# Flux shell integration for bash.
# Emits OSC 7 (cwd) and OSC 133 (prompt/command lifecycle) sequences
# so Flux can track the shell state.

__flux_command_finished() {
    local exit_code=$?
    printf '\x1b]133;D;%d\x1b\\' "$exit_code"
    return "$exit_code"
}

__flux_prompt_start() {
    printf '\x1b]133;A\x1b\\'
}

__flux_command_start() {
    printf '\x1b]133;C\x1b\\'
}

__flux_update_cwd() {
    printf '\x1b]7;file://%s%s\x1b\\' "$HOSTNAME" "$PWD"
}

# Wrap PROMPT_COMMAND to emit sequences on each prompt.
__flux_original_prompt_command="${PROMPT_COMMAND:-}"
PROMPT_COMMAND='__flux_command_finished; __flux_prompt_start; __flux_update_cwd; eval "${__flux_original_prompt_command}"'

# Use DEBUG trap for preexec (command start).
trap '__flux_command_start' DEBUG
