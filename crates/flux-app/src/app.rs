//! Application state and event handling.
//!
//! `App` owns the window, renderer, PTY, and terminal state; the rest
//! of this module is an impl-spread across sibling files — each file
//! adds its own `impl App` block for a focused slice of behavior.
//! Fields are `pub(crate)` so siblings can read and mutate them
//! directly without ceremony.

mod clipboard;
mod display;
mod initialize;
mod keyboard;
mod layout;
mod mouse;
mod popup;
mod scroll;
mod search;
mod tabs;
mod terminal_events;

use std::sync::Arc;

use arboard::Clipboard;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use flux_input::{Autocomplete, CommandHistory, InputEditor};
use flux_terminal::state::TerminalState;

use crate::config::FluxConfig;
use crate::keys::Keymap;
use crate::mux::{MuxState, Pane};

pub(crate) use popup::PopupState;

/// Minimum rows reserved for the input bar: top rule + one input line
/// + bottom rule.
pub(crate) const MIN_INPUT_BAR_ROWS: usize = 3;

/// Height of the custom titlebar strip in logical pixels (macOS
/// unified chrome; zero elsewhere — native decorations remain).
pub(crate) const TITLEBAR_LOGICAL_H: f32 = if cfg!(target_os = "macos") { 34.0 } else { 0.0 };

/// Application state — owns the window, renderer, PTY, and terminal state.
pub struct App {
    pub(crate) config: FluxConfig,
    /// Chord → action bindings: platform defaults + `[keys]`.
    pub(crate) keymap: Keymap,
    pub(crate) proxy: winit::event_loop::EventLoopProxy<()>,
    pub(crate) window: Option<Arc<Window>>,
    pub(crate) renderer: Option<flux_renderer::Renderer>,
    /// Tabs and panes (#40); every keystroke, PTY read, and render
    /// goes through the focused pane.
    pub(crate) mux: MuxState,
    /// Shell history loaded at startup — cloned into each new pane's
    /// editor. (Live cross-pane history sync is future work.)
    pub(crate) base_history: CommandHistory,
    /// True when a full-screen program (vim, less, fzf) owns the keyboard.
    /// When set, keystrokes route directly to the PTY and Flux's input
    /// chrome collapses to zero.
    pub(crate) raw_mode: bool,
    /// Current keyboard modifier state, tracked via `ModifiersChanged` events.
    /// Needed for clipboard shortcuts (Cmd+V / Ctrl+Shift+V) since winit's
    /// `KeyEvent` doesn't carry modifier state directly.
    pub(crate) modifiers: ModifiersState,
    /// System clipboard handle. Lazily created so a clipboard init failure
    /// doesn't take down the whole app on startup.
    pub(crate) clipboard: Option<Clipboard>,
    /// Active overlay, if any. R6 introduces the field with only the
    /// `Hidden` variant; F7 / F14 add autocomplete and search intercepts
    /// that read this to decide whether to swallow a keystroke.
    pub(crate) popup: PopupState,
    pub(crate) autocomplete: Autocomplete,
    /// Find-in-scrollback bar state (F14).
    pub(crate) search: search::SearchBar,
    /// Armed close confirmation: (what, id) and when (#58).
    pub(crate) close_confirm: Option<((&'static str, u64), std::time::Instant)>,
    /// Sidebar shown? (Cmd+Shift+B toggles; `[sidebar] visible` seeds.)
    pub(crate) sidebar_visible: bool,
    /// Last sidebar state pushed to the renderer — skip identical
    /// rebuilds during output floods.
    pub(crate) last_sidebar: Option<(Vec<flux_renderer::SidebarEntry>, usize, u32, u32)>,
    /// Fractional scroll remainder from trackpad pixel deltas — whole
    /// lines are consumed per wheel event, the rest accumulates here.
    pub(crate) scroll_accum: f32,
    /// Set when the last tab's shell exits (PTY EOF) — the event loop
    /// shuts the app down on the next wake. A shell exiting in one of
    /// several tabs just closes that tab.
    pub(crate) shell_exited: bool,
    /// Click/drag tracking for mouse selection (F12). The selection
    /// itself lives in `TerminalState` (content-anchored, survives
    /// scrolling) — see mouse.rs.
    pub(crate) mouse: mouse::MouseState,
}

impl App {
    /// The focused pane's terminal, if any pane exists yet.
    pub(crate) fn terminal(&self) -> Option<&TerminalState> {
        self.mux.focused_pane().map(|pane| &pane.terminal)
    }

    /// The focused pane (PTY + terminal + editor together — one
    /// borrow, so callers can use all sides without fighting the
    /// borrow checker).
    pub(crate) fn pane_mut(&mut self) -> Option<&mut Pane> {
        self.mux.focused_pane_mut()
    }

    /// The focused pane's input editor.
    pub(crate) fn input_mut(&mut self) -> Option<&mut InputEditor> {
        self.mux.focused_pane_mut().map(|pane| &mut pane.input)
    }

    /// Immutable view of the focused pane's editor.
    pub(crate) fn input_ref(&self) -> Option<&InputEditor> {
        self.mux.focused_pane().map(|pane| &pane.input)
    }

    /// A fresh editor for a new pane, seeded with the shell history.
    pub(crate) fn new_editor(&self) -> InputEditor {
        InputEditor::with_history(self.base_history.clone())
    }

    pub fn new(
        config: FluxConfig,
        proxy: winit::event_loop::EventLoopProxy<()>,
        history: CommandHistory,
    ) -> Self {
        let keymap = Keymap::defaults().with_overrides(&config.keys);
        let sidebar_visible = config.sidebar.visible;
        Self {
            sidebar_visible,
            last_sidebar: None,
            keymap,
            config,
            proxy,
            window: None,
            renderer: None,
            mux: MuxState::new(),
            base_history: history,
            raw_mode: false,
            modifiers: ModifiersState::empty(),
            clipboard: None,
            popup: PopupState::Hidden,
            autocomplete: Autocomplete::default(),
            search: search::SearchBar::default(),
            close_confirm: None,
            scroll_accum: 0.0,
            shell_exited: false,
            mouse: mouse::MouseState::default(),
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

            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_mouse_wheel(delta);
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.handle_mouse_moved(position);
            }

            WindowEvent::MouseInput { state, button, .. } => {
                self.handle_mouse_input(state, button);
            }

            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, _event: ()) {
        // PTY output arrived — process and redraw
        self.process_pty_output();
        if self.shell_exited {
            log::info!("Shell exited — closing window");
            event_loop.exit();
            return;
        }
        self.request_redraw();
    }
}
