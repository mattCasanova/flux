//! Fish shell implementation.

use std::path::{Path, PathBuf};
use crate::{Shell, InjectionMethod};

pub struct Fish {
    binary: PathBuf,
}

impl Fish {
    pub fn new(binary: PathBuf) -> Self {
        Self { binary }
    }
}

impl Shell for Fish {
    fn binary(&self) -> &Path {
        &self.binary
    }

    fn name(&self) -> &str {
        "fish"
    }

    fn history_file(&self) -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/share"))
            .join("fish/fish_history")
    }

    fn parse_history_entry(&self, raw_line: &str) -> Option<String> {
        // Fish history format:
        // - cmd: ls -la
        //   when: 1712700000
        if let Some(cmd) = raw_line.strip_prefix("- cmd: ") {
            Some(cmd.to_string())
        } else {
            None
        }
    }

    fn integration_script(&self) -> &str {
        // TODO: Load from shell/flux-integration.fish
        ""
    }

    fn injection_method(&self) -> InjectionMethod {
        InjectionMethod::RcFile {
            rc_path: dirs::config_dir()
                .unwrap_or_else(|| dirs::home_dir().unwrap().join(".config"))
                .join("fish/config.fish"),
            source_line: "test -f ~/.config/flux/shell/flux-integration.fish; and source ~/.config/flux/shell/flux-integration.fish".into(),
        }
    }

    fn rc_files(&self) -> Vec<PathBuf> {
        vec![
            dirs::config_dir()
                .unwrap_or_else(|| dirs::home_dir().unwrap().join(".config"))
                .join("fish/config.fish"),
        ]
    }

    fn spawn_args(&self) -> Vec<String> {
        vec!["--login".into(), "--interactive".into()]
    }
}
