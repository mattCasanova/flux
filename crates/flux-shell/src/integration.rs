//! Shell integration scripts — embedded at compile time.
//!
//! Each script emits OSC 7 (cwd tracking) and OSC 133 (prompt/command
//! lifecycle) sequences. zsh loads invisibly via the ZDOTDIR bootstrap
//! (nothing typed into the shell, nothing echoed at startup); bash and
//! fish are still auto-sourced by writing into the PTY.

pub const ZSH_INTEGRATION: &str = include_str!("../resources/flux-integration.zsh");
pub const BASH_INTEGRATION: &str = include_str!("../resources/flux-integration.bash");
pub const FISH_INTEGRATION: &str = include_str!("../resources/flux-integration.fish");

/// ZDOTDIR bootstrap template. Contains `__FLUX_INTEGRATION_PATH__`,
/// which the caller replaces with the absolute path of the written
/// integration script.
pub const ZSH_BOOTSTRAP_TEMPLATE: &str = include_str!("../resources/flux-bootstrap.zshenv");
