//! The local machine as a [`Domain`] — spawns the user's shell in a
//! PTY via `PtyManager`, with the environment the shell-integration
//! bootstrap needs (ZDOTDIR redirection etc.) applied to every pane.

use std::path::PathBuf;

use anyhow::Result;

use crate::domain::{Domain, DomainId, Pty};
use crate::pty::{PtyManager, WakeCallback};

pub struct LocalDomain {
    id: DomainId,
    shell: PathBuf,
    /// Extra environment for every spawned pane (e.g. the ZDOTDIR
    /// bootstrap that loads shell integration invisibly).
    extra_env: Vec<(String, String)>,
}

impl LocalDomain {
    pub fn new(id: DomainId, shell: PathBuf, extra_env: Vec<(String, String)>) -> Self {
        Self {
            id,
            shell,
            extra_env,
        }
    }
}

impl Domain for LocalDomain {
    fn spawn_pane(&self, cols: u16, rows: u16, wake: WakeCallback) -> Result<Box<dyn Pty + Send>> {
        let pty = PtyManager::spawn(
            self.shell.to_str().unwrap_or("/bin/zsh"),
            cols,
            rows,
            wake,
            &self.extra_env,
        )?;
        Ok(Box::new(pty))
    }

    fn id(&self) -> DomainId {
        self.id
    }

    fn name(&self) -> &str {
        "local"
    }
}
