//! PTY management — spawns shells and handles I/O.
//!
//! Wraps portable-pty to provide:
//! - Shell spawning with proper environment setup
//! - Non-blocking reads from PTY output
//! - Writing user input to PTY
//! - Terminal resize handling

use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;

use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

/// Messages from the PTY read thread to the main thread.
pub enum PtyEvent {
    /// New output bytes from the shell.
    Output(Vec<u8>),
    /// The shell process exited.
    Exited,
}

/// Manages a PTY connection to a shell process.
pub struct PtyManager {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    event_rx: mpsc::Receiver<PtyEvent>,
    cols: u16,
    rows: u16,
}

impl PtyManager {
    /// Spawn a shell in a new PTY.
    pub fn spawn(shell_path: &str, cols: u16, rows: u16) -> Result<Self> {
        let pty_system = native_pty_system();

        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        // Set up the shell command
        let mut cmd = CommandBuilder::new(shell_path);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("LANG", "en_US.UTF-8");

        // Spawn the shell
        let _child = pair.slave.spawn_command(cmd)?;
        // Drop the slave — the child owns it now
        drop(pair.slave);

        // Get handles for reading and writing
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        // Spawn reader thread — sends output to main thread via channel
        let (event_tx, event_rx) = mpsc::channel();
        Self::spawn_reader_thread(reader, event_tx);

        log::info!("PTY spawned: {} ({}x{})", shell_path, cols, rows);

        Ok(Self {
            master: pair.master,
            writer,
            event_rx,
            cols,
            rows,
        })
    }

    /// Read any pending PTY output. Non-blocking — returns empty vec if nothing new.
    pub fn read_events(&self) -> Vec<PtyEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Write bytes to the PTY (user input).
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Resize the PTY (called when window size changes).
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.cols = cols;
        self.rows = rows;
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Spawn the background reader thread.
    fn spawn_reader_thread(
        mut reader: Box<dyn Read + Send>,
        tx: mpsc::Sender<PtyEvent>,
    ) {
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let _ = tx.send(PtyEvent::Exited);
                        break;
                    }
                    Ok(n) => {
                        let _ = tx.send(PtyEvent::Output(buf[..n].to_vec()));
                    }
                    Err(e) => {
                        log::error!("PTY read error: {}", e);
                        let _ = tx.send(PtyEvent::Exited);
                        break;
                    }
                }
            }
        });
    }
}
