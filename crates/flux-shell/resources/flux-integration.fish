# Flux shell integration for fish.
# Emits OSC 7 (cwd) and OSC 133 (prompt/command lifecycle) sequences
# so Flux can track the shell state.

function __flux_command_start --on-event fish_preexec
    printf '\x1b]133;C\x1b\\'
end

function __flux_command_finished --on-event fish_postexec
    printf '\x1b]133;D;%d\x1b\\' $status
end

function __flux_prompt_start --on-event fish_prompt
    printf '\x1b]133;A\x1b\\'
end

function __flux_update_cwd --on-event fish_postexec
    printf '\x1b]7;file://%s%s\x1b\\' (hostname) "$PWD"
end
