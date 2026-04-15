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
//!
//! # Evolution
//!
//! R3 (this file) introduces an empty `BlockCapture`. Every `Perform`
//! callback is the trait default no-op — we don't forward or handle
//! anything yet. F4 extends it to parse OSC 7 for cwd tracking; F8 adds
//! OSC 133 for block state. The file intentionally has no visible effect
//! today — the whole point is to land the scaffolding first so F4/F8 are
//! a small incremental PR rather than a big one.

use alacritty_terminal::vte::Perform;

/// Side-channel OSC interceptor. See module docs for the design rationale.
#[derive(Default)]
pub(crate) struct BlockCapture {
    // R3: empty. F4 adds `cwd: Option<PathBuf>`. F8 adds block state.
}

impl BlockCapture {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

impl Perform for BlockCapture {
    // R3: all callbacks inherit the trait's default no-op bodies.
    // F4 overrides `osc_dispatch` to catch OSC 7 (cwd).
    // F8 extends `osc_dispatch` to catch OSC 133 (prompt/command/exit).
    //
    // We intentionally do NOT override print/execute/csi_dispatch/etc. —
    // the main alacritty parser handles all of those. Our parser exists
    // purely to see OSC sequences that alacritty's ansi layer drops.
}
