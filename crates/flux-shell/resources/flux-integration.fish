# Flux shell integration for fish.
# Emits OSC 7 (cwd) and OSC 133 (prompt/command lifecycle) sequences
# so Flux can track the shell state.
#
# Lifecycle markers: A = prompt start, B = prompt end (input begins),
# C = command execution start, D;code = command finished.

# Idempotency: sourcing twice (auto-inject + user config) must not
# double-register hooks or double-wrap fish_prompt.
if not set -q __FLUX_INTEGRATION_LOADED
    set -g __FLUX_INTEGRATION_LOADED 1

    function __flux_command_start --on-event fish_preexec
        printf '\x1b]133;C\x1b\\'
    end

    function __flux_command_finished --on-event fish_postexec
        printf '\x1b]133;D;%d\x1b\\' $status
    end

    # cwd on every directory change (not just after commands, so the
    # first prompt and `cd` inside scripts are covered too).
    function __flux_update_cwd --on-variable PWD
        # string escape --style=url percent-encodes everything reserved,
        # including '/', which must stay literal in the path part.
        printf '\x1b]7;file://%s%s\x1b\\' (hostname) \
            (string escape --style=url -- "$PWD" | string replace -a '%2F' '/')
    end

    # Wrap the prompt function so A/B hug the rendered prompt exactly.
    functions -c fish_prompt __flux_original_fish_prompt
    function fish_prompt
        printf '\x1b]133;A\x1b\\'
        __flux_original_fish_prompt
        printf '\x1b]133;B\x1b\\'
    end

    # Emit the initial cwd for the very first prompt.
    __flux_update_cwd
end
