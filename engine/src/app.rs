//! Window-side application state: the winit event handler, keyboard
//! mapping, signal handling, and the shared exit/control flags exchanged
//! with the render thread.

use crate::hud;
use crate::frame_loop::render_main;
use sim::flight::Controls;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use sim::flight::SIM_STEP;
use winit::application::ApplicationHandler;
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

/// Custom user event forwarded from background worker threads to the winit event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UserEvent {
    /// Render thread has completed all work and shut down.
    RenderDone,
}

/// Global exit flag set by POSIX signal handlers (SIGINT / SIGTERM).
static SIGNAL_EXIT: AtomicBool = AtomicBool::new(false);

pub(crate) extern "C" fn handle_signal(_sig: libc::c_int) {
    if SIGNAL_EXIT.swap(true, Ordering::SeqCst) {
        // Second signal: force immediate termination
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            libc::raise(libc::SIGINT);
        }
    }
}

/// Thread-safe state shared between the main UI event thread and the rendering thread.
pub(crate) struct Shared {
    /// Atomic exit flag signaled by window close, ESC/Ctrl+C, or SIGINT.
    pub(crate) exit: AtomicBool,
    /// Bitmask of currently depressed flight control keys.
    pub(crate) keys: AtomicU32,
    pub(crate) ui: AtomicU32,
}

impl Shared {
    /// Check whether exit has been requested either via application event or POSIX signal.
    #[inline]
    pub(crate) fn should_exit(&self) -> bool {
        self.exit.load(Ordering::Acquire) || SIGNAL_EXIT.load(Ordering::Acquire)
    }

    /// Request application shutdown across all threads.
    #[inline]
    pub(crate) fn request_exit(&self) {
        self.exit.store(true, Ordering::Release);
        SIGNAL_EXIT.store(true, Ordering::Release);
    }
}

const KEY_W: u32 = 1;
const KEY_S: u32 = 1 << 1;
const KEY_A: u32 = 1 << 2;
const KEY_D: u32 = 1 << 3;
const KEY_Q: u32 = 1 << 4;
const KEY_E: u32 = 1 << 5;
const KEY_SHIFT: u32 = 1 << 6;

/// Map a physical keycode into its corresponding bitmask flag.
pub(crate) fn key_bit(code: KeyCode) -> u32 {
    match code {
        KeyCode::KeyW | KeyCode::ArrowUp => KEY_W,
        KeyCode::KeyS | KeyCode::ArrowDown => KEY_S,
        KeyCode::KeyA | KeyCode::ArrowLeft => KEY_A,
        KeyCode::KeyD | KeyCode::ArrowRight => KEY_D,
        KeyCode::KeyQ => KEY_Q,
        KeyCode::KeyE => KEY_E,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => KEY_SHIFT,
        _ => 0,
    }
}

/// Decode the current pressed-key bitmask into normalized flight control inputs.
pub(crate) fn controls_from(bits: u32) -> Controls {
    Controls {
        pitch: (if bits & KEY_S != 0 { 1.0 } else { 0.0 })
            - (if bits & KEY_W != 0 { 1.0 } else { 0.0 }),
        bank: (if bits & KEY_A != 0 { 1.0 } else { 0.0 })
            - (if bits & KEY_D != 0 { 1.0 } else { 0.0 }),
        yaw: (if bits & KEY_E != 0 { 1.0 } else { 0.0 })
            - (if bits & KEY_Q != 0 { 1.0 } else { 0.0 }),
        boost: bits & KEY_SHIFT != 0,
    }
}

/// Main winit application handler managing window creation, input routing, and render thread lifecycle.
pub(crate) struct App {
    pub(crate) window: Option<std::sync::Arc<Window>>,
    pub(crate) shared: std::sync::Arc<Shared>,
    pub(crate) proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    pub(crate) render_thread: Option<std::thread::JoinHandle<()>>,
    pub(crate) ctrl_pressed: bool,
}
impl ApplicationHandler<UserEvent> for App {
    /// Window creation and render thread startup when the application is resumed.
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("explora")
            .with_inner_size(winit::dpi::LogicalSize::new(1920, 1080))
            .with_maximized(!crate::flags::start_windowed())
            .with_fullscreen(if std::env::args().any(|a| a == "--fullscreen") {
                Some(winit::window::Fullscreen::Borderless(None))
            } else {
                None
            });
        let window = std::sync::Arc::new(event_loop.create_window(attrs).expect("window"));
        let thread_window = window.clone();
        let shared = self.shared.clone();
        let proxy = self.proxy.clone();
        self.window = Some(window);
        self.render_thread = Some(std::thread::spawn(move || {
            render_main(thread_window, shared, proxy);
        }));
    }

    /// Handle window-level input events: close requests, fullscreen toggle (F11), and flight controls.
    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.shared.request_exit();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.ctrl_pressed = modifiers.state().control_key();
            }
            WindowEvent::Focused(false) => {
                self.shared.keys.store(0, Ordering::Relaxed);
                self.ctrl_pressed = false;
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key,
                        logical_key,
                        state,
                        repeat,
                        ..
                    },
                ..
            } => {
                let pressed = state.is_pressed();

                // Track physical Control key presses
                if let PhysicalKey::Code(code) = physical_key {
                    if code == KeyCode::ControlLeft || code == KeyCode::ControlRight {
                        self.ctrl_pressed = pressed;
                    }
                }

                // Exit requested via ESC shortcut or Ctrl+C combination
                let is_esc = matches!(logical_key, Key::Named(NamedKey::Escape))
                    || matches!(physical_key, PhysicalKey::Code(KeyCode::Escape));
                let is_ctrl_c = (matches!(physical_key, PhysicalKey::Code(KeyCode::KeyC))
                    && self.ctrl_pressed)
                    || matches!(&logical_key, Key::Character(s) if s == "\x03");

                if pressed && (is_esc || is_ctrl_c) {
                    self.shared.request_exit();
                    return;
                }

                if let PhysicalKey::Code(code) = physical_key {
                    let bit = key_bit(code);
                    if pressed {
                        if !repeat {
                            let toggle = match code {
                                KeyCode::KeyH => hud::VISIBLE,
                                KeyCode::F1 => hud::HELP,
                                KeyCode::KeyM => hud::AUDIO,
                                KeyCode::KeyP => hud::PAUSED,
                                _ => 0,
                            };
                            self.shared.ui.fetch_xor(toggle, Ordering::Relaxed);
                            if code == KeyCode::KeyR { self.shared.ui.fetch_or(hud::RESET, Ordering::Relaxed); }
                        }
                        if code == KeyCode::F11 && !repeat {
                            if let Some(window) = self.window.as_ref() {
                                let full = window.fullscreen().is_none();
                                window.set_fullscreen(
                                    full.then(|| winit::window::Fullscreen::Borderless(None)),
                                );
                            }
                        }
                        self.shared.keys.fetch_or(bit, Ordering::Relaxed);
                    } else {
                        self.shared.keys.fetch_and(!bit, Ordering::Relaxed);
                    }
                }
            }
            _ => {}
        }
        let _ = event_loop;
    }

    /// Process user events forwarded from background threads.
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        if event == UserEvent::RenderDone {
            if let Some(handle) = self.render_thread.take() {
                let _ = handle.join();
            }
            event_loop.exit();
        }
    }

    /// Put event thread to sleep when no UI events are pending, or exit if shutdown requested.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.shared.should_exit() {
            event_loop.exit();
        } else {
            // Event thread sleeps. The render thread never waits on it.
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

/// Consume whole physics ticks without throwing away interpolation phase.
/// The caller caps incoming frame time at 100 ms to bound catch-up work.
pub(crate) fn simulation_steps(accumulator: &mut f32) -> u32 {
    let steps = (*accumulator / SIM_STEP).floor() as u32;
    *accumulator -= steps as f32 * SIM_STEP;
    steps
}

