# Flux — 1.21 gigawatts of terminal

> Where we're going, we don't need Electron.

**Stop using a terminal that feels like it's from 1955.**

Flux is a GPU-accelerated, open-source terminal emulator written in Rust —
built for the way developers actually work in 2025. Command blocks. Inline
autocomplete. Multi-line editing. Native shell integration. Tabs, splits,
and persistent sessions baked in, no tmux required. No telemetry. No
account. No cloud. No flux capacitor required.

Your terminal emulator should take you back to the future.

> ⚠️ **Early development.** Flux is pre-alpha. Lots of foundational features
> aren't built yet (multi-line input, command history, autocomplete, shell
> integration, blocks, tabs). It works as a basic terminal today and we're
> shipping toward a real daily-driver foundation in v0.2. See
> [`plans/feature-roadmap.md`](https://github.com/mattCasanova/flux/blob/master/plans/feature-roadmap.md)
> for the full picture.

## Why another terminal?

The Warp-style "smart terminal" niche — command blocks, inline autocomplete,
multi-line editing, native UI for SSH and panes — isn't filled by an open
source project yet. Alacritty, Kitty, WezTerm, and Ghostty are great GPU
terminals but they're closer to the iTerm2 model than the Warp model. Warp
itself is closed source, sends telemetry, and requires an account.

Flux is the gap-filler: Warp's UX, fully open source, no telemetry, no
account, runs entirely on your machine.

## Status

| Capability | Status |
|---|---|
| GPU-rendered terminal grid | ✅ working |
| Multi-shell support (zsh, bash, fish) | ✅ working |
| Per-style glyph atlas (regular/bold/italic) | ✅ working |
| Vim, less, nano, htop, fzf via raw mode | ✅ working |
| Clipboard paste (Cmd+V) | ✅ working |
| Single-line input editor with `❯` prompt | ✅ working |
| Multi-line input editor | 🚧 v0.2 |
| Command history (up/down) | 🚧 v0.2 |
| Inline autocomplete | 🚧 v0.2 |
| Shell integration (OSC 7 / OSC 133) | 🚧 v0.2 |
| Command blocks | 🚧 v0.3 |
| Tabs and split panes | 🚧 v0.4 |
| Mux daemon (close GUI, come back later) | 🚧 v0.5 |
| SSH (spawn-in-PTY + auto-tmux) | 🚧 v0.5 |
| Native SSH client (russh) | 🚧 v1.0 |
| Kitty keyboard + graphics protocols | 🚧 v1.0 |

See [`plans/feature-roadmap.md`](plans/feature-roadmap.md) for the full
roadmap and the per-milestone implementation docs in `plans/v0.2-*.md`
through `plans/v1.x-*.md`.

## Building from source

You need a Rust toolchain (1.75+ recommended). Install via
[rustup](https://rustup.rs/) if you don't have one.

### Install directly from GitHub

```bash
cargo install --git https://github.com/mattCasanova/flux flux-app
```

This compiles Flux and installs the `flux` binary to `~/.cargo/bin/`. Make
sure `~/.cargo/bin` is on your `PATH`. Run with:

```bash
flux
```

### Build from a local clone

```bash
git clone https://github.com/mattCasanova/flux.git
cd flux
cargo build --release
./target/release/flux
```

For development work, use `cargo run --release` (faster glyph rendering
than the debug build):

```bash
cargo run --release
```

### Platform support

- **macOS** — Apple Silicon and Intel, primary development target
- **Linux** — should work, less tested
- **Windows** — not yet, planned for v1.0+

## Configuration

Flux looks for a config file at `~/.config/flux/config.toml`. On first run
it creates a default one based on
[`resources/default-config.toml`](resources/default-config.toml). Edit
that file to change the font, theme, window size, and padding. The full
config schema lands in v0.2 — see
[`plans/v0.2-daily-driver.md`](https://github.com/mattCasanova/flux/blob/master/plans/v0.2-daily-driver.md)
for what's coming.

Current configurable options:

- `font.family` — any monospace font on your system
- `font.size` — in points
- `font.weight` — `"normal"` or `"bold"`
- `font.style` — `"normal"` or `"italic"`
- `font.line_height` — multiplier (default `1.0`)
- `window.title`, `window.width`, `window.height`
- `window.padding_horizontal`, `window.padding_vertical`
- `theme.background`, `theme.foreground` — hex colors

## Architecture

Flux is a Cargo workspace with six crates:

```
crates/
├── flux-types/       — shared data types (Color, CellData, RenderGrid)
├── flux-renderer/    — wgpu rendering pipeline, glyph atlas, instance buffer
├── flux-terminal/    — wraps alacritty_terminal + portable-pty
├── flux-shell/       — shell detection (zsh, bash, fish)
├── flux-input/       — input editor buffer
└── flux-app/         — the binary that wires everything together
```

The renderer uses **instanced cell quads** (single draw call for the entire
visible grid), a **per-style glyph atlas** (4 styles cached independently),
and **bottom-anchored output rendering** (new content enters from the
bottom, scrolls up). See
[`plans/architecture/gpu-rendering.md`](plans/architecture/gpu-rendering.md)
for the full pipeline write-up.

## Roadmap

Six version milestones, each shippable on its own:

- **v0.2 — Daily Driver Foundation** (next): clipboard copy, multi-line input,
  command history, autocomplete, shell integration, scrollback, search,
  configurable keybindings, themes
- **v0.3 — Blocks** (the killer Warp differentiator)
- **v0.4 — Multi-pane** (tabs + splits via the Domain trait)
- **v0.5 — Persistence + Remote** (mux daemon + first SSH paths)
- **v1.0 — Native SSH + Advanced Protocols** (russh, Kitty keyboard, Kitty
  graphics, OSC 8 hyperlinks, status bar, public launch)
- **v1.x — Polish Pass** (vim rendering, font metrics, settings GUI, file
  tree, profiling)

See [`plans/feature-roadmap.md`](plans/feature-roadmap.md) for the full
detail and [GitHub Milestones](https://github.com/mattCasanova/flux/milestones)
for live progress.

## Contributing

Pre-1.0, the project is in heavy flux (no pun intended) and breaking
changes happen often. If you want to contribute, the best path right now is:

1. Pick something from the [v0.2 milestone](https://github.com/mattCasanova/flux/milestone/2)
2. Comment on the issue saying you're working on it
3. Read the corresponding section in `plans/v0.2-daily-driver.md` for the
   implementation design
4. Open a PR

A formal `CONTRIBUTING.md` lands closer to v1.0 alongside CI, code of
conduct, and issue templates. For now, just be friendly and we'll figure
it out together.

## License

MIT — see [LICENSE](LICENSE) (coming soon — workspace already declares MIT).

## Tagline options

- "Stop using a terminal that feels like it's from 1955."
- "Your terminal should take you back to the future."
- "1.21 gigawatts of GPU-rendered terminal."
- "Where we're going, we don't need Electron."
- "Warp-style blocks. Open source. Zero telemetry."
- "Make like a tree and get out of the 80s."

The project is named after the flux capacitor. The default window title
is "Flux — 1.21 gigawatts." Yes, we're committed to the bit.
