# Flux shell integration for zsh.
# Emits OSC 7 (cwd) and OSC 133 (prompt/command lifecycle) sequences
# so Flux can track the shell state.

__flux_command_finished() {
    local exit_code=$?
    printf '\x1b]133;D;%d\x1b\\' "$exit_code"
}

__flux_prompt_start() {
    printf '\x1b]133;A\x1b\\'
}

__flux_command_start() {
    printf '\x1b]133;C\x1b\\'
}

__flux_update_cwd() {
    printf '\x1b]7;file://%s%s\x1b\\' "$HOST" "$PWD"
}

# Order matters: command_finished MUST run first so $? is still the
# previous command's exit code before any other precmd hook overwrites it.
precmd_functions=(__flux_command_finished __flux_prompt_start __flux_update_cwd "${precmd_functions[@]}")
preexec_functions+=(__flux_command_start)
