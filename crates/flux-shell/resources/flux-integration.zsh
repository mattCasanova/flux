# Flux shell integration for zsh.
# Emits OSC 7 (cwd) and OSC 133 (prompt/command lifecycle) sequences
# so Flux can track the shell state.
#
# Lifecycle markers: A = prompt start, B = prompt end (input begins),
# C = command execution start, D;code = command finished.
# A and B wrap PROMPT itself (zero-width via %{...%}) so they hug the
# rendered prompt exactly; D/C come from precmd/preexec hooks.

# Idempotency: sourcing twice (auto-inject + user rc) must not
# double-register hooks or double-wrap the prompt.
[[ -n ${__FLUX_INTEGRATION_LOADED:-} ]] && return 0
typeset -g __FLUX_INTEGRATION_LOADED=1

__flux_command_finished() {
    local exit_code=$?
    printf '\x1b]133;D;%d\x1b\\' "$exit_code"
}

__flux_command_start() {
    printf '\x1b]133;C\x1b\\'
}

__flux_update_cwd() {
    # Percent-encode ASCII characters outside the URL-safe set. Raw
    # multibyte UTF-8 passes through unchanged — Flux's decoder handles
    # it — but an unencoded space or '%' would corrupt the URL.
    local encoded='' ch
    for ch in ${(s::)PWD}; do
        if [[ $ch == [A-Za-z0-9/._~-] ]]; then
            encoded+=$ch
        elif (( #ch > 127 )); then
            encoded+=$ch
        else
            printf -v ch '%%%02X' $(( #ch ))
            encoded+=$ch
        fi
    done
    printf '\x1b]7;file://%s%s\x1b\\' "$HOST" "$encoded"
}

# Wrap PROMPT with A (start) and B (end) markers. Re-checked every
# prompt because themes (powerlevel10k etc.) rewrite PROMPT from their
# own precmd hooks; runs LAST in precmd_functions so it wraps the
# final prompt.
__flux_wrap_prompt() {
    [[ $PROMPT == *$'\x1b]133;B'* ]] && return 0
    PROMPT=$'%{\x1b]133;A\x1b\\%}'"$PROMPT"$'%{\x1b]133;B\x1b\\%}'
}

# Order matters: __flux_command_finished MUST run first so $? is still
# the previous command's exit code before any other precmd hook
# overwrites it; __flux_wrap_prompt MUST run last (see above).
precmd_functions=(__flux_command_finished __flux_update_cwd "${precmd_functions[@]}" __flux_wrap_prompt)
preexec_functions+=(__flux_command_start)

# Emit cwd immediately so the very first prompt has it (the first
# precmd only fires after the first command otherwise... it does fire
# before the first prompt, but a cd in zshrc after sourcing would be
# missed until then).
__flux_update_cwd
