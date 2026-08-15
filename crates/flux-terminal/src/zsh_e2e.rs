//! End-to-end check of the semantic stream against a real `zsh`.
//!
//! The unit tests in `state.rs` fake the OSC 133 byte stream. This one
//! spawns `/bin/zsh -i` on a PTY with Flux's actual integration script
//! (loaded through the ZDOTDIR bootstrap, exactly as the app does),
//! runs a few commands, and checks that `TerminalState` ends up with
//! the spans, rows and exit codes a user would see. Skips (passes) if
//! there is no `/bin/zsh`.

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::state::TerminalState;
use flux_types::ResolvedTheme;

const ZSH: &str = "/bin/zsh";
const OVERALL_TIMEOUT: Duration = Duration::from_secs(15);

/// Runs `commands` one prompt at a time and returns the terminal state
/// after the shell exits.
fn drive_zsh(commands: &[&str]) -> Option<TerminalState> {
    if !std::path::Path::new(ZSH).exists() {
        eprintln!("no {ZSH}; skipping");
        return None;
    }

    // Hermetic HOME + ZDOTDIR bootstrap, mirroring App::write_zsh_bootstrap.
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let zdot = dir.path().join("zdot");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&zdot).unwrap();
    let script = dir.path().join("flux-integration.zsh");
    std::fs::write(&script, flux_shell::integration::ZSH_INTEGRATION).unwrap();
    let bootstrap = flux_shell::integration::ZSH_BOOTSTRAP_TEMPLATE
        .replace("__FLUX_INTEGRATION_PATH__", &script.display().to_string());
    std::fs::write(zdot.join(".zshenv"), bootstrap).unwrap();
    // A fixed-width prompt (`%~` would not collapse to `~` here: the
    // temp HOME sits behind macOS's /var → /private/var symlink).
    std::fs::write(home.join(".zshrc"), "PS1='%# '\n").unwrap();

    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(pair) => pair,
        // Sandboxed runners deny the pty device; that's an environment
        // limit, not a Flux bug.
        Err(e) => {
            eprintln!("openpty unavailable ({e}); skipping");
            return None;
        }
    };
    let mut cmd = CommandBuilder::new(ZSH);
    cmd.arg("-i");
    cmd.env("HOME", &home);
    cmd.env("ZDOTDIR", &zdot);
    cmd.env("TERM", "xterm-256color");
    cmd.env_remove("FLUX_ORIG_ZDOTDIR");
    cmd.cwd(&home);
    let mut child = pair.slave.spawn_command(cmd).expect("spawn zsh");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
    let mut pending = commands.iter().chain(std::iter::once(&"exit"));
    // One command per prompt: send when a prompt appears, then wait
    // for it to be consumed (live prompt gone) before the next.
    let mut sent_for_this_prompt = false;
    let started = Instant::now();
    loop {
        if started.elapsed() > OVERALL_TIMEOUT {
            let _ = child.kill();
            panic!("zsh e2e timed out; spans so far: {:?}", state.debug_spans());
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(bytes) => {
                state.process_bytes(&bytes);
                let at_prompt = state.tracker_live_prompt().is_some();
                if !at_prompt {
                    sent_for_this_prompt = false;
                } else if !sent_for_this_prompt && let Some(cmd) = pending.next() {
                    writer.write_all(format!("{cmd}\r").as_bytes()).unwrap();
                    writer.flush().unwrap();
                    sent_for_this_prompt = true;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = child.wait();
    Some(state)
}

#[test]
fn real_zsh_session_yields_spans_rows_and_exit_codes() {
    let Some(state) = drive_zsh(&["true", "false", "printf 'x\\ny\\n'"]) else {
        return;
    };
    let spans = state.debug_spans();
    // three closed cycles; `exit` never reaches a new prompt.
    assert!(spans.len() >= 3, "expected ≥3 spans, got {spans:?}");
    let closed: Vec<_> = spans.iter().filter(|s| s.is_closed()).collect();
    assert_eq!(closed.len(), 3, "three closed spans: {spans:?}");

    let (t, f, p) = (closed[0], closed[1], closed[2]);
    assert_eq!(t.exit_code, Some(0), "true → 0");
    assert_eq!(f.exit_code, Some(1), "false → 1");
    assert_eq!(p.exit_code, Some(0), "printf → 0");

    // Header rows carry the prompt + echo; output rows are exactly the
    // two printf lines.
    let header = state.debug_row_text_abs(t.prompt_start);
    assert!(header.ends_with("true"), "true header row: {header:?}");
    let header = state.debug_row_text_abs(f.prompt_start);
    assert!(header.ends_with("false"), "false header row: {header:?}");
    let out_start = p.output_start.expect("printf has output");
    assert_eq!(
        p.end.unwrap() - out_start,
        2,
        "printf: two output rows: {p:?}"
    );
    assert_eq!(state.debug_row_text_abs(out_start), "x");
    assert_eq!(state.debug_row_text_abs(out_start + 1), "y");
    // Prompt end sits after `% ` (col 2) on the header row.
    assert_eq!(t.prompt_end.map(|(r, _)| r), Some(t.prompt_start));
    assert_eq!(t.prompt_end.map(|(_, c)| c), Some(2));
    // Each header is exactly one row: C fired on the row after the echo.
    assert_eq!(t.output_start, Some(t.prompt_start + 1));
}
