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
mod popup;
mod terminal_events;

use std::sync::Arc;

use arboard::Clipboard;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use flux_input::InputEditor;
use flux_terminal::pty::PtyManager;
use flux_terminal::state::TerminalState;

use crate::config::FluxConfig;

pub(crate) use popup::PopupState;

/// Rows reserved below the output grid for Flux chrome:
/// one divider row plus one input editor row.
pub(crate) const INPUT_CHROME_ROWS: usize = 2;

/// Application state — owns the window, renderer, PTY, and terminal state.
pub struct App {
    pub(crate) config: FluxConfig,
    pub(crate) proxy: winit::event_loop::EventLoopProxy<()>,
    pub(crate) window: Option<Arc<Window>>,
    pub(crate) renderer: Option<flux_renderer::Renderer>,
    pub(crate) pty: Option<PtyManager>,
    pub(crate) terminal: Option<TerminalState>,
    pub(crate) input: InputEditor,
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
            popup: PopupState::Hidden,
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
