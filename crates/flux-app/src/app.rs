//! Application state and event handling.

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use arboard::Clipboard;
use winit::keyboard::ModifiersState;

use flux_input::InputEditor;
use flux_terminal::pty::{PtyEvent, PtyManager};
use flux_terminal::state::{TermEvent, TerminalState};
use flux_types::Color;

use crate::config::FluxConfig;

/// Rows reserved below the output grid for Flux chrome:
/// one divider row plus one input editor row.
const INPUT_CHROME_ROWS: usize = 2;

/// Application state — owns the window, renderer, PTY, and terminal state.
pub struct App {
    config: FluxConfig,
    proxy: winit::event_loop::EventLoopProxy<()>,
    window: Option<Arc<Window>>,
    renderer: Option<flux_renderer::Renderer>,
    pty: Option<PtyManager>,
    terminal: Option<TerminalState>,
    input: InputEditor,
    /// True when a full-screen program (vim, less, fzf) owns the keyboard.
    /// When set, keystrokes route directly to the PTY and Flux's input
    /// chrome collapses to zero.
    raw_mode: bool,
    /// Current keyboard modifier state, tracked via `ModifiersChanged` events.
    /// Needed for clipboard shortcuts (Cmd+V / Ctrl+Shift+V) since winit's
    /// `KeyEvent` doesn't carry modifier state directly.
    modifiers: ModifiersState,
    /// System clipboard handle. Lazily created so a clipboard init failure
    /// doesn't take down the whole app on startup.
    clipboard: Option<Clipboard>,
}

impl App {
    pub fn new(config: FluxConfig, proxy: winit::event_loop::EventLoopProxy<()>) -> Self {
        Self {
            config,
            proxy,
            window: None,
            renderer: None,
            pty: None,
            terminal: None,
            input: InputEditor::new(),
            raw_mode: false,
            modifiers: ModifiersState::empty(),
            clipboard: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        if let Err(e) = self.initialize(event_loop) {
            log::error!("Failed to initialize: {}", e);
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                self.handle_resize(size.width, size.height);
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.handle_scale_change(scale_factor as f32);
            }

            WindowEvent::RedrawRequested => {
                self.handle_redraw();
            }

            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                self.handle_keyboard(event);
            }

            WindowEvent::ModifiersChanged(new) => {
                self.modifiers = new.state();
            }

            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        // PTY output arrived — process and redraw
        self.process_pty_output();
        self.request_redraw();
    }
}

// --- Private helpers ---

impl App {
    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let window_attrs = Window::default_attributes()
            .with_title(&self.config.window.title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.config.window.width,
                self.config.window.height,
            ));

        let window = Arc::new(event_loop.create_window(window_attrs)?);
        let mut renderer = self.create_renderer(&window)?;

        // Apply padding from config
        let scale_factor = window.scale_factor() as f32;
        let pad_x = self.config.window.padding_horizontal * scale_factor;
        let pad_y = self.config.window.padding_vertical * scale_factor;
        renderer.set_padding(pad_x, pad_y);

        // Calculate grid dimensions from window size, padding, and cell metrics.
        // Reserve `INPUT_CHROME_ROWS` rows at the bottom for the divider + input editor.
        let metrics = renderer.cell_metrics();
        let inner_size = window.inner_size();
        let usable_w = (inner_size.width as f32 - pad_x * 2.0).max(0.0);
        let usable_h = (inner_size.height as f32 - pad_y * 2.0).max(0.0);
        let cols = (usable_w / metrics.width) as usize;
        let total_rows = (usable_h / metrics.height) as usize;
        let rows = total_rows.saturating_sub(INPUT_CHROME_ROWS);
        log::info!("Grid: {}x{} (padding {}x{}, chrome {} rows)", cols, rows, pad_x, pad_y, INPUT_CHROME_ROWS);

        // Create terminal state
        let terminal = TerminalState::new(cols.max(1), rows.max(1));

        // Spawn the PTY with matching dimensions
        let shell = flux_shell::detect_shell();
        let proxy = self.proxy.clone();
        let wake = Box::new(move || {
            let _ = proxy.send_event(());
        });
        let pty = PtyManager::spawn(
            shell.binary().to_str().unwrap_or("/bin/zsh"),
            cols.max(1) as u16,
            rows.max(1) as u16,
            wake,
        )?;

        log::info!("Renderer + PTY initialized");
        self.renderer = Some(renderer);
        self.terminal = Some(terminal);
        self.pty = Some(pty);
        self.window = Some(window);

        self.update_display();
        self.update_input_display();
        self.request_redraw();

        Ok(())
    }

    fn create_renderer(
        &self,
        window: &Arc<Window>,
    ) -> anyhow::Result<flux_renderer::Renderer> {
        let scale_factor = window.scale_factor() as f32;
        let font_size_px = self.config.font.size * scale_factor;

        let mut renderer = flux_renderer::Renderer::new(
            window.clone(),
            &self.config.font.family,
            font_size_px,
            self.config.font.line_height,
            self.default_glyph_style(),
        )?;

        if let Some(bg) = Color::from_hex(&self.config.theme.background) {
            renderer.set_clear_color(bg);
        }

        Ok(renderer)
    }

    /// Default glyph style applied to cells with no BOLD/ITALIC flag —
    /// read from `[font] weight` and `[font] style` in the config file.
    fn default_glyph_style(&self) -> flux_renderer::GlyphStyle {
        let bold = self.config.font.weight.eq_ignore_ascii_case("bold");
        let italic = self.config.font.style.eq_ignore_ascii_case("italic");
        flux_renderer::GlyphStyle::from_flags(bold, italic)
    }

    /// Process pending PTY output through alacritty_terminal.
    fn process_pty_output(&mut self) {
        let Some(pty) = &self.pty else { return };
        let Some(terminal) = &mut self.terminal else { return };

        let mut dirty = false;

        for event in pty.read_events() {
            match event {
                PtyEvent::Output(bytes) => {
                    terminal.process_bytes(&bytes);
                    dirty = true;
                }
                PtyEvent::Exited => {
                    log::info!("Shell exited");
                }
            }
        }

        // Handle events from alacritty_terminal (PtyWrite responses)
        if dirty {
            for event in terminal.drain_events() {
                match event {
                    TermEvent::PtyWrite(text) => {
                        if let Some(pty) = &mut self.pty {
                            let _ = pty.write(text.as_bytes());
                        }
                    }
                    TermEvent::Title(title) => {
                        if let Some(window) = &self.window {
                            window.set_title(&title);
                        }
                    }
                    TermEvent::Bell => {
                        log::debug!("Bell");
                    }
                }
            }

            // Raw-mode state can change on any PTY output (vim enters alt
            // screen on launch, fzf flips termios, etc.). Re-check before
            // rendering the next frame.
            self.sync_raw_mode();
            self.update_display();
        }
    }

    /// Detect whether a full-screen program is on the other end of the PTY
    /// and, if the state just changed, resize the grid and toggle chrome.
    ///
    /// Uses `TermMode::ALT_SCREEN` as the sole signal — vim, less, man, htop,
    /// tmux, fzf (default) and top all set the alt-screen bit. We deliberately
    /// do NOT check termios here: every interactive shell (zsh zle, bash
    /// readline, fish) keeps the PTY in termios-raw mode whenever it's ready
    /// for input, so `tcgetattr` is a false-positive trap — it fires as soon
    /// as the shell prints its first prompt. Password prompts and other
    /// termios-only raw-mode programs that skip alt-screen are a follow-up
    /// (tracked separately).
    fn sync_raw_mode(&mut self) {
        let Some(terminal) = &self.terminal else { return };
        let raw = terminal.is_alt_screen();
        if raw == self.raw_mode {
            return;
        }
        self.raw_mode = raw;
        log::info!("Raw mode: {}", raw);

        if let Some(renderer) = &mut self.renderer {
            renderer.set_bottom_anchor(!raw);
            renderer.set_show_shell_cursor(raw);
        }

        // Recompute the grid dimensions so alt-screen programs get every
        // row, and restore the 2-row chrome when they exit.
        self.apply_window_layout();

        if raw {
            if let Some(renderer) = &mut self.renderer {
                renderer.hide_input_line();
            }
        } else {
            self.update_input_display();
        }
    }

    /// Recompute the grid dimensions from the current window size, accounting
    /// for padding and whether Flux chrome is currently reserving rows. Called
    /// on startup, window resize, scale change, and raw-mode transitions.
    fn apply_window_layout(&mut self) {
        let Some(window) = &self.window else { return };
        let Some(renderer) = &mut self.renderer else { return };

        let inner_size = window.inner_size();
        let metrics = renderer.cell_metrics();
        let pad_x = self.padding_x();
        let pad_y = self.padding_y();
        let usable_w = (inner_size.width as f32 - pad_x * 2.0).max(0.0);
        let usable_h = (inner_size.height as f32 - pad_y * 2.0).max(0.0);
        let cols = (usable_w / metrics.width) as usize;
        let total_rows = (usable_h / metrics.height) as usize;
        let chrome_rows = if self.raw_mode { 0 } else { INPUT_CHROME_ROWS };
        let rows = total_rows.saturating_sub(chrome_rows).max(1);

        if let Some(terminal) = &mut self.terminal {
            terminal.resize(cols.max(1), rows);
        }
        if let Some(pty) = &mut self.pty {
            let _ = pty.resize(cols.max(1) as u16, rows as u16);
        }
    }

    fn padding_x(&self) -> f32 {
        let scale_factor = self.window.as_ref().map(|w| w.scale_factor() as f32).unwrap_or(1.0);
        self.config.window.padding_horizontal * scale_factor
    }

    fn padding_y(&self) -> f32 {
        let scale_factor = self.window.as_ref().map(|w| w.scale_factor() as f32).unwrap_or(1.0);
        self.config.window.padding_vertical * scale_factor
    }

    /// Render the terminal grid.
    fn update_display(&mut self) {
        let Some(terminal) = &self.terminal else { return };
        let Some(renderer) = &mut self.renderer else { return };

        let fg = Color::from_hex(&self.config.theme.foreground).unwrap_or(Color::default());
        let bg = Color::from_hex(&self.config.theme.background)
            .unwrap_or(Color::new(0.0, 0.0, 0.0, 1.0));

        let grid = terminal.render_grid(fg, bg);
        renderer.set_grid(&grid);
    }

    /// Push the current input editor state to the renderer.
    fn update_input_display(&mut self) {
        let Some(renderer) = &mut self.renderer else { return };
        renderer.set_input_line(self.input.buffer(), self.input.cursor_col());
    }

    fn handle_keyboard(&mut self, event: winit::event::KeyEvent) {
        use winit::event::ElementState;

        if event.state != ElementState::Pressed {
            return;
        }

        // Clipboard shortcuts run ahead of mode-specific handling so they
        // work identically in cooked and raw mode. Cmd on macOS maps to
        // super in winit; Ctrl+Shift is the cross-platform fallback.
        if self.is_paste_shortcut(&event) {
            self.handle_paste();
            return;
        }

        if self.raw_mode {
            self.handle_keyboard_raw(event);
            return;
        }

        self.handle_keyboard_cooked(event);
    }

    /// Detect the system paste chord — Cmd+V on macOS, Ctrl+Shift+V elsewhere.
    fn is_paste_shortcut(&self, event: &winit::event::KeyEvent) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let is_v = matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("v"))
            || matches!(&event.logical_key, Key::Named(NamedKey::Paste));
        if !is_v {
            return false;
        }
        let m = self.modifiers;
        if cfg!(target_os = "macos") {
            m.super_key() && !m.control_key() && !m.alt_key()
        } else {
            m.control_key() && m.shift_key() && !m.alt_key() && !m.super_key()
        }
    }

    /// Read the system clipboard and route the text into the editor (cooked
    /// mode) or the PTY (raw mode). In raw mode we wrap the payload in the
    /// bracketed-paste markers when the child program has enabled that mode,
    /// so vim et al can distinguish a paste from typed input.
    fn handle_paste(&mut self) {
        let text = match self.clipboard_text() {
            Some(t) if !t.is_empty() => t,
            _ => return,
        };

        if self.raw_mode {
            let bracketed = self
                .terminal
                .as_ref()
                .map(|t| t.is_bracketed_paste())
                .unwrap_or(false);
            if let Some(pty) = &mut self.pty {
                if bracketed {
                    let _ = pty.write(b"\x1b[200~");
                }
                let _ = pty.write(text.as_bytes());
                if bracketed {
                    let _ = pty.write(b"\x1b[201~");
                }
            }
        } else {
            // In cooked mode we collapse newlines so multi-line pastes don't
            // fire submissions through Enter handling. Proper multi-line
            // editing lands with #22.
            let sanitized: String = text
                .chars()
                .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
                .collect();
            self.input.insert_str(&sanitized);
            self.update_input_display();
        }

        self.request_redraw();
    }

    fn clipboard_text(&mut self) -> Option<String> {
        if self.clipboard.is_none() {
            match Clipboard::new() {
                Ok(cb) => self.clipboard = Some(cb),
                Err(e) => {
                    log::error!("Clipboard init failed: {}", e);
                    return None;
                }
            }
        }
        match self.clipboard.as_mut()?.get_text() {
            Ok(text) => Some(text),
            Err(e) => {
                log::warn!("Clipboard read failed: {}", e);
                None
            }
        }
    }

    /// Cooked-mode key handling — keystrokes go through the Flux input editor,
    /// Enter submits the composed line, Ctrl+<letter> bypasses the editor.
    fn handle_keyboard_cooked(&mut self, event: winit::event::KeyEvent) {
        use winit::keyboard::{Key, NamedKey};
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

        match &event.logical_key {
            // Enter submits the composed line to the PTY (plus \r).
            Key::Named(NamedKey::Enter) => {
                let line = self.input.take_line();
                if let Some(pty) = &mut self.pty {
                    let _ = pty.write(line.as_bytes());
                    let _ = pty.write(b"\r");
                }
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::Backspace) => {
                self.input.backspace();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::Delete) => {
                self.input.delete_forward();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::ArrowLeft) => {
                self.input.move_left();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.input.move_right();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::Home) => {
                self.input.home();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::End) => {
                self.input.end();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            // Arrow up/down are reserved for history (#21) — swallow for now so
            // they don't bleed into the PTY as cursor movements.
            Key::Named(NamedKey::ArrowUp) | Key::Named(NamedKey::ArrowDown) => return,
            _ => {}
        }

        // Everything else: text input or Ctrl+<letter>. `text_with_all_modifiers`
        // folds Ctrl effects into the string (Ctrl+C → \x03, Ctrl+D → \x04, etc.),
        // which is the terminal-correct interpretation. Single-byte control
        // characters bypass the editor and go straight to the PTY; anything else
        // is insertable text.
        let Some(text) = event.text_with_all_modifiers() else { return };
        if text.is_empty() {
            return;
        }

        let is_control = text.len() == 1 && (text.as_bytes()[0] < 0x20 || text.as_bytes()[0] == 0x7f);
        if is_control {
            // Ctrl+C clears the editor buffer so the user starts fresh after the interrupt.
            if text.as_bytes()[0] == 0x03 {
                self.input.clear();
                self.update_input_display();
            }
            if let Some(pty) = &mut self.pty {
                let _ = pty.write(text.as_bytes());
            }
        } else {
            self.input.insert_str(text);
            self.update_input_display();
        }

        self.request_redraw();
    }

    /// Raw-mode key handling — the PTY owns the keyboard. Forward named keys
    /// as the standard xterm escape sequences and everything else via
    /// `text_with_all_modifiers` so Ctrl combos land correctly.
    fn handle_keyboard_raw(&mut self, event: winit::event::KeyEvent) {
        use winit::keyboard::{Key, NamedKey};
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

        let bytes: Option<&[u8]> = match &event.logical_key {
            Key::Named(NamedKey::Enter) => Some(b"\r"),
            Key::Named(NamedKey::Backspace) => Some(b"\x7f"),
            Key::Named(NamedKey::Tab) => Some(b"\t"),
            Key::Named(NamedKey::Escape) => Some(b"\x1b"),
            Key::Named(NamedKey::ArrowUp) => Some(b"\x1b[A"),
            Key::Named(NamedKey::ArrowDown) => Some(b"\x1b[B"),
            Key::Named(NamedKey::ArrowRight) => Some(b"\x1b[C"),
            Key::Named(NamedKey::ArrowLeft) => Some(b"\x1b[D"),
            Key::Named(NamedKey::Home) => Some(b"\x1b[H"),
            Key::Named(NamedKey::End) => Some(b"\x1b[F"),
            Key::Named(NamedKey::PageUp) => Some(b"\x1b[5~"),
            Key::Named(NamedKey::PageDown) => Some(b"\x1b[6~"),
            Key::Named(NamedKey::Delete) => Some(b"\x1b[3~"),
            _ => None,
        };

        if let Some(bytes) = bytes {
            if let Some(pty) = &mut self.pty {
                let _ = pty.write(bytes);
            }
        } else if let Some(text) = event.text_with_all_modifiers() {
            if let Some(pty) = &mut self.pty {
                let _ = pty.write(text.as_bytes());
            }
        }

        self.request_redraw();
    }

    fn handle_resize(&mut self, width: u32, height: u32) {
        // Reconfigure surface + resize grid + render — all in the same event.
        // Presenting a frame before returning from the resize handler prevents
        // the compositor from stretching a stale frame.
        if let Some(renderer) = &mut self.renderer {
            renderer.resize(width, height);
        }

        self.apply_window_layout();
        self.update_display();
        if !self.raw_mode {
            self.update_input_display();
        }

        let renderer = self.renderer.as_mut().expect("renderer not initialized");
        if let Err(e) = renderer.render() {
            log::error!("Resize render error: {}", e);
        }
    }

    fn handle_redraw(&mut self) {
        let Some(renderer) = &mut self.renderer else { return };
        if let Err(e) = renderer.render() {
            log::error!("Render error: {}", e);
        }
    }

    fn handle_scale_change(&mut self, scale_factor: f32) {
        log::info!("Scale factor changed to {}", scale_factor);

        let font_size_px = self.config.font.size * scale_factor;
        let font_family = self.config.font.family.clone();
        let line_height = self.config.font.line_height;

        let Some(renderer) = &mut self.renderer else { return };
        if let Err(e) = renderer.rebuild_font(&font_family, font_size_px, line_height) {
            log::error!("Failed to rebuild font: {}", e);
            return;
        }

        // Recalculate grid after font change
        if let Some(window) = &self.window {
            let size = window.inner_size();
            renderer.resize(size.width, size.height);
        }

        self.apply_window_layout();
        self.update_display();
        if !self.raw_mode {
            self.update_input_display();
        }
        self.request_redraw();
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
