//! Zsh shell implementation.

use std::path::{Path, PathBuf};
use crate::{Shell, InjectionMethod};

pub struct Zsh {
    binary: PathBuf,
}

impl Zsh {
    pub fn new(binary: PathBuf) -> Self {
        Self { binary }
    }
}

impl Shell for Zsh {
    fn binary(&self) -> &Path {
        &self.binary
    }

    fn name(&self) -> &str {
        "zsh"
    }

    fn history_file(&self) -> PathBuf {
        std::env::var("HISTFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs::home_dir().unwrap().join(".zsh_history"))
    }

    fn parse_history_entry(&self, raw_line: &str) -> Option<String> {
        // Zsh extended history format: ": 1712700000:0;ls -la"
        if raw_line.starts_with(": ") {
            raw_line.splitn(2, ';').nth(1).map(|s| s.to_string())
        } else {
            Some(raw_line.to_string())
        }
    }

    fn integration_script(&self) -> &str {
        // TODO: Load from shell/flux-integration.zsh
        ""
    }

    fn injection_method(&self) -> InjectionMethod {
        InjectionMethod::RcFile {
            rc_path: dirs::home_dir().unwrap().join(".zshrc"),
            source_line: r#"[ -f ~/.config/flux/shell/flux-integration.zsh ] && source ~/.config/flux/shell/flux-integration.zsh"#.into(),
        }
    }

    fn rc_files(&self) -> Vec<PathBuf> {
        let home = dirs::home_dir().unwrap();
        vec![
            home.join(".zshenv"),
            home.join(".zprofile"),
            home.join(".zshrc"),
            home.join(".zlogin"),
        ]
    }

    fn spawn_args(&self) -> Vec<String> {
        vec!["--login".into(), "--interactive".into()]
    }
}
