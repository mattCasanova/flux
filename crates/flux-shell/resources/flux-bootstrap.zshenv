# Flux zsh bootstrap — loaded because Flux spawned zsh with ZDOTDIR
# pointing at this directory. Restores the user's real ZDOTDIR, chains
# their own .zshenv, and sources the Flux integration — all before the
# first prompt, with nothing typed into the shell (so nothing echoes).

if [[ -n "${FLUX_ORIG_ZDOTDIR:-}" ]]; then
    export ZDOTDIR="$FLUX_ORIG_ZDOTDIR"
    unset FLUX_ORIG_ZDOTDIR
else
    unset ZDOTDIR
fi

# Chain the user's own .zshenv, if any.
if [[ -f "${ZDOTDIR:-$HOME}/.zshenv" ]]; then
    builtin source "${ZDOTDIR:-$HOME}/.zshenv"
fi

# Load the integration for interactive shells. Hooks self-order on the
# first preexec, so themes registered later (oh-my-zsh, p10k) can't
# displace the exit-code capture or the prompt-marker wrap.
if [[ -o interactive ]]; then
    builtin source "__FLUX_INTEGRATION_PATH__"
fi
