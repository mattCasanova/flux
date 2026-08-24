# Flux — 1.21 gigawatts of terminal

> Where we're going, we don't need Electron.

Flux is a GPU-accelerated terminal emulator written in Rust with the
Warp-style workflow — command blocks, a real input editor, tabs and
splits, a sidebar that knows what each tab is doing — and none of the
Warp-style strings attached. **No telemetry. No account. No cloud.**
Everything runs and stays on your machine.

## Install

One line, no Rust toolchain needed. Detects macOS or Linux, Apple
Silicon/arm64 or x86_64, and installs the latest release into
`~/.local/bin`:

```sh
curl -fsSL https://raw.githubusercontent.com/mattCasanova/flux/master/install.sh | sh
```

Then run `flux`. Releases live on the
[Releases page](https://github.com/mattCasanova/flux/releases) with
checksums. Prefer building from source? `cargo install --git
https://github.com/mattCasanova/flux flux-app` (Rust stable).

> **Early days.** Flux is pre-1.0 and its author dogfoods it daily on
> macOS. Linux builds ship and pass CI but have had little desktop time
> yet (Omarchy/Hyprland is the first target). Expect rough edges; bug
> reports are very welcome (see below).

## What it does today (v0.4)

- **Blocks.** Every command is a unit: your own prompt becomes the
  block header, failures are marked red with the exit code, durations
  show on the header, a sticky header pins the command while its
  output scrolls. Click a block to select it, `Cmd+↑/↓` to walk
  blocks, `Cmd+C` to copy one, `Cmd+Shift+C` to copy the last output,
  double-click a header to re-run it. Blocks survive `clear`, resizes,
  and splits.
- **Input editor.** Multi-line editing (`Shift+Enter`), history recall,
  path autocomplete (`Tab`), undo/redo. The shell's own prompt is hidden
  while you type — the input box *is* the prompt — and reappears as the
  block header once the command runs.
- **Tabs and splits.** `Cmd+T` tabs, `Cmd+D` / `Cmd+Shift+D` splits,
  each pane a self-contained terminal with its own input box.
- **Sidebar.** Tabs as a left panel showing the current folder, git
  branch, and a running-command indicator per tab. `Cmd+B` toggles it.
- **Find in scrollback** (`Cmd+F`), mouse selection across scrollback,
  a scrollbar while scrolled, full-screen programs (vim, less, htop,
  Claude Code) with mouse forwarding.
- **Shell integration** installed invisibly for zsh (nothing typed,
  nothing echoed); bash and fish get a visible fallback for now.
- **Everything themed from one file** — the terminal palette and every
  chrome color (sidebar, input box, titlebar, popups, tints), plus every
  keyboard shortcut, in `~/.config/flux/config.toml`.

## Shortcuts (macOS defaults; all rebindable under `[keys]`)

| | |
|---|---|
| `Cmd+T` / `Cmd+W` / `Cmd+1–9` / `Cmd+[` `Cmd+]` | new / close / jump / cycle tabs |
| `Cmd+D` / `Cmd+Shift+D` / `Cmd+Shift+W` / `Cmd+Alt+arrows` | split right / down / close pane / focus pane |
| `Cmd+B` | toggle sidebar |
| `Cmd+F` | find (`Enter`/`↓` next, `Shift+Enter`/`↑` previous, `Esc`) |
| `Cmd+↑` / `Cmd+↓` | select previous / next block |
| `Cmd+C` / `Cmd+Shift+C` / `Cmd+V` | copy selection or block / copy last output / paste |
| `Cmd+Z` / `Cmd+Shift+Z` | undo / redo in the input box |
| `PageUp` / `PageDown`, `Alt+↑/↓` | scroll |

On Linux the defaults are `Ctrl+Shift`-based (placeholders until the
Linux keymap is designed around the super key). The generated config
lists every action.

## Configuration

Created on first run at `~/.config/flux/config.toml`, fully commented:
`[font]`, `[window]` (size, padding), `[theme]` (palette),
`[theme.ui]` (every chrome color, individually), `[scrollback]`,
`[blocks]`, `[sidebar]`, `[keys]`.

## Bugs and feedback

Flux sends nothing anywhere. Logs and crash dumps stay local in
`~/.local/state/flux/`. To report a bug, open an
[issue](https://github.com/mattCasanova/flux/issues) with the version
line from `flux --version` and, if relevant, the latest log file from
that folder — you can read exactly what you're attaching.

## Roadmap

Done: GPU grid, daily-driver editor, blocks, tabs, splits, sidebar,
themes, keybindings, releases. Next: the keymap design pass (macOS +
Linux super-key layouts), sidebar v2 (close buttons, agent awareness —
is Claude in that tab busy or idle?), then persistence and remote
sessions (mux daemon, SSH with integration), then a native SSH client
and the Kitty protocols for 1.0. Live progress:
[issues](https://github.com/mattCasanova/flux/issues).

## Architecture

A Cargo workspace of six crates:

```
crates/
├── flux-types/       shared data types, resolved theme
├── flux-renderer/    wgpu pipeline: instanced cell quads, glyph atlas, chrome
├── flux-terminal/    alacritty_terminal + portable-pty, block tracking, search
├── flux-shell/       shell detection + integration scripts (zsh, bash, fish)
├── flux-input/       the input editor (buffer, history, undo, autocomplete)
└── flux-app/         window, event loop, mux (tabs/panes), config, keymap
```

The whole UI — grid, sidebar, titlebar, input boxes, popups — is one
instanced draw over a glyph atlas. Terminal emulation is
`alacritty_terminal`; Flux's own layer on top tracks command blocks by
absolute scrollback row and re-anchors them through reflow by content.

## Contributing

It's a solo, long-horizon project and forks are welcome. Issues with
real reproduction steps are the most useful thing you can send. A
`CONTRIBUTING.md` will come with 1.0.

## License

MIT — see [LICENSE](LICENSE).

---

The project is named after the flux capacitor. The default window title
is "Flux — 1.21 gigawatts." Yes, we're committed to the bit.
