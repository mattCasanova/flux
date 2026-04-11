//! Bash shell implementation.

use std::path::{Path, PathBuf};
use crate::{Shell, InjectionMethod};

pub struct Bash {
    binary: PathBuf,
}

impl Bash {
    pub fn new(binary: PathBuf) -> Self {
        Self { binary }
    }
}

impl Shell for Bash {
    fn binary(&self) -> &Path {
        &self.binary
    }

    fn name(&self) -> &str {
        "bash"
    }

    fn history_file(&self) -> PathBuf {
        std::env::var("HISTFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs::home_dir().unwrap().join(".bash_history"))
    }

    fn parse_history_entry(&self, raw_line: &str) -> Option<String> {
        // Bash timestamps are on separate lines prefixed with #
        if raw_line.starts_with('#') {
            None
        } else {
            Some(raw_line.to_string())
        }
    }

    fn integration_script(&self) -> &str {
        // TODO: Load from shell/flux-integration.bash
        ""
    }

    fn injection_method(&self) -> InjectionMethod {
        InjectionMethod::RcFile {
            rc_path: dirs::home_dir().unwrap().join(".bashrc"),
            source_line: r#"[ -f ~/.config/flux/shell/flux-integration.bash ] && source ~/.config/flux/shell/flux-integration.bash"#.into(),
        }
    }

    fn rc_files(&self) -> Vec<PathBuf> {
        let home = dirs::home_dir().unwrap();
        vec![
            home.join(".bash_profile"),
            home.join(".bashrc"),
        ]
    }

    fn spawn_args(&self) -> Vec<String> {
        vec!["--login".into()]
    }
}
