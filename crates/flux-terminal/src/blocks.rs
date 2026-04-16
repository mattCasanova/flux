//! Side-channel OSC interceptor that runs in parallel with alacritty's
//! `ansi::Processor`.
//!
//! # Why a side parser
//!
//! We need to see OSC 7 (cwd) and OSC 133 (prompt / command / exit code)
//! sequences so Flux can implement shell integration and command blocks.
//! The obvious approach — "wrap `Term<EventProxy>` as a `vte::Perform` and
//! forward everything" — does not work, for three compounding reasons:
//!
//! 1. `vte::ansi::Processor::advance<H>` requires `H: Handler`, not
//!    `H: Perform`. The trait shape is wrong for a wrapper.
//! 2. `Term` implements `Handler`, not `Perform` — there's no user-facing
//!    `Term::osc_dispatch` to forward to. A transparent wrapper would need
//!    to re-implement all ~80 `Handler` methods.
//! 3. Even with the right trait, the `Performer` inside `vte::ansi` only
//!    dispatches a short whitelist of OSCs (0/2/4/8/10/11/12/22/50/52/
//!    104/110/111/112). OSC 7 and OSC 133 fall through to `unhandled` and
//!    are dropped before they ever reach the `Handler`.
//!
//! The fix is to run two parsers. Alacritty's main `ansi::Processor`
//! stays exactly as it is today — driving `Term`, updating the grid. In
//! parallel, we feed the same PTY byte stream into a stock `vte::Parser`
//! driving this `BlockCapture` impl. Because `vte::Parser` is the raw
//! parser-level interface that fires *before* alacritty's ansi filtering,
//! OSC 7 / OSC 133 arrive verbatim in our `osc_dispatch`.
//!
//! # Cost
//!
//! `vte::Parser::advance` is a tight per-byte state machine. For typical
//! PTY throughput (a few MB/s peak during `cat` of a large file) the side
//! parser adds sub-microsecond overhead per KB — negligible next to
//! alacritty's grid updates or the renderer's per-frame work.

use std::path::{Path, PathBuf};

use alacritty_terminal::vte::Perform;
use percent_encoding::percent_decode_str;

/// Side-channel OSC interceptor. See module docs for the design rationale.
#[derive(Default)]
pub(crate) struct BlockCapture {
    /// The shell's current working directory, updated on each OSC 7
    /// sequence. `None` until the first OSC 7 arrives.
    cwd: Option<PathBuf>,
}

impl BlockCapture {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    /// Parse an OSC 7 sequence: `\x1b]7;file://hostname/path\x07`
    ///
    /// `params[0]` is `b"7"`, `params[1]` is `b"file://hostname/path"`.
    /// We strip the hostname (everything before the first `/` after
    /// `file://`) and URL-decode the path.
    fn handle_osc_7(&mut self, params: &[&[u8]]) {
        let Some(url) = params.get(1).and_then(|b| std::str::from_utf8(b).ok()) else {
            return;
        };
        let Some(rest) = url.strip_prefix("file://") else { return };
        let Some(path_start) = rest.find('/') else { return };
        let encoded = &rest[path_start..];
        let decoded = percent_decode_str(encoded).decode_utf8_lossy();
        self.cwd = Some(PathBuf::from(decoded.into_owned()));
    }
}

impl Perform for BlockCapture {
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell: bool) {
        let Some(&first) = params.first() else { return };
        if first == b"7" {
            self.handle_osc_7(params);
        }
        // F8 extends this match for OSC 133.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::Parser;

    /// Feed a raw byte sequence through the vte parser into BlockCapture
    /// and return the resulting cwd.
    fn parse_cwd(input: &[u8]) -> Option<PathBuf> {
        let mut capture = BlockCapture::new();
        let mut parser = Parser::new();
        parser.advance(&mut capture, input);
        capture.cwd().map(PathBuf::from)
    }

    #[test]
    fn basic_osc_7() {
        let cwd = parse_cwd(b"\x1b]7;file://localhost/tmp\x07");
        assert_eq!(cwd.as_deref(), Some(Path::new("/tmp")));
    }

    #[test]
    fn osc_7_with_spaces() {
        let cwd = parse_cwd(b"\x1b]7;file://localhost/home/user/my%20folder\x07");
        assert_eq!(cwd.as_deref(), Some(Path::new("/home/user/my folder")));
    }

    #[test]
    fn osc_7_no_hostname() {
        let cwd = parse_cwd(b"\x1b]7;file:///Users/matt/src\x07");
        assert_eq!(cwd.as_deref(), Some(Path::new("/Users/matt/src")));
    }

    #[test]
    fn osc_7_with_st_terminator() {
        // String Terminator (\x1b\\) instead of BEL (\x07)
        let cwd = parse_cwd(b"\x1b]7;file://localhost/tmp\x1b\\");
        assert_eq!(cwd.as_deref(), Some(Path::new("/tmp")));
    }

    #[test]
    fn osc_7_updates_on_cd() {
        let mut capture = BlockCapture::new();
        let mut parser = Parser::new();
        parser.advance(&mut capture, b"\x1b]7;file://localhost/home\x07");
        assert_eq!(capture.cwd(), Some(Path::new("/home")));
        parser.advance(&mut capture, b"\x1b]7;file://localhost/tmp\x07");
        assert_eq!(capture.cwd(), Some(Path::new("/tmp")));
    }

    #[test]
    fn no_osc_7_means_none() {
        let cwd = parse_cwd(b"hello world\n");
        assert_eq!(cwd, None);
    }
}
