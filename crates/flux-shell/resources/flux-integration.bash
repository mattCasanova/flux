# Flux shell integration for bash.
# Emits OSC 7 (cwd) and OSC 133 (prompt/command lifecycle) sequences
# so Flux can track the shell state.
#
# Lifecycle markers: A = prompt start, B = prompt end (input begins),
# C = command execution start, D;code = command finished.

# Idempotency: sourcing twice (auto-inject + user rc) must not
# double-register hooks or double-wrap the prompt.
[ -n "${__FLUX_INTEGRATION_LOADED:-}" ] && return 0
__FLUX_INTEGRATION_LOADED=1

__flux_command_finished() {
    local exit_code=$?
    printf '\x1b]133;D;%d\x1b\\' "$exit_code"
    return "$exit_code"
}

__flux_update_cwd() {
    # Percent-encode ASCII characters outside the URL-safe set; raw
    # multibyte UTF-8 passes through (Flux's decoder handles it).
    local encoded='' i ch code
    for ((i = 0; i < ${#PWD}; i++)); do
        ch=${PWD:i:1}
        case "$ch" in
            [A-Za-z0-9/._~-]) encoded+=$ch ;;
            *)
                code=$(printf '%d' "'$ch")
                if [ "$code" -gt 127 ]; then
                    encoded+=$ch
                else
                    printf -v ch '%%%02X' "$code"
                    encoded+=$ch
                fi
                ;;
        esac
    done
    printf '\x1b]7;file://%s%s\x1b\\' "$HOSTNAME" "$encoded"
}

# Wrap PS1 with A (start) and B (end) markers, zero-width via \[ \].
# Re-checked every prompt in case user config rewrites PS1. NOTE: bash
# prompt expansion understands \e but NOT \x1b.
__flux_wrap_ps1() {
    case "$PS1" in
        *'133;B'*) ;;
        *) PS1="\[\e]133;A\e\\\\\]${PS1}\[\e]133;B\e\\\\\]" ;;
    esac
}

# preexec via DEBUG trap, guarded bash-preexec style: the trap fires
# before EVERY simple command — including the parts of PROMPT_COMMAND
# itself — so an unguarded trap would emit C while sitting at the
# prompt and leave Flux thinking a command runs forever. The
# __flux_at_prompt flag is set as the LAST step of PROMPT_COMMAND and
# consumed by the first DEBUG fire after it: exactly one C per
# submitted command line.
__flux_at_prompt=0
__flux_preexec() {
    [ "$__flux_at_prompt" != 1 ] && return 0
    [ -n "${COMP_LINE:-}" ] && return 0     # readline completion, not a command
    __flux_at_prompt=0
    printf '\x1b]133;C\x1b\\'
}
trap '__flux_preexec' DEBUG

__flux_original_prompt_command="${PROMPT_COMMAND:-}"
PROMPT_COMMAND='__flux_command_finished; __flux_update_cwd; eval "${__flux_original_prompt_command}"; __flux_wrap_ps1; __flux_at_prompt=1'

# First prompt: cwd available immediately.
__flux_update_cwd
