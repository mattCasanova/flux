//! Shell integration scripts — embedded at compile time.
//!
//! Each script emits OSC 7 (cwd tracking) and OSC 133 (prompt/command
//! lifecycle) sequences. Auto-sourced by Flux on PTY spawn.

pub const ZSH_INTEGRATION: &str = include_str!("../resources/flux-integration.zsh");
pub const BASH_INTEGRATION: &str = include_str!("../resources/flux-integration.bash");
pub const FISH_INTEGRATION: &str = include_str!("../resources/flux-integration.fish");
