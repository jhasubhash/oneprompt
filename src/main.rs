//! oneprompt: a terminal where the background and the type are part of the design.
//!
//! The shell runs in a PTY managed by alacritty_terminal (see terminal.rs);
//! this file wires window events to it and asks the renderer to draw the grid
//! with the current theme.
//!
//! Usage:
//!   oneprompt                 run your login shell
//!   oneprompt -e "command"    run a command through /bin/sh instead
//!   ONEPROMPT_SNAPSHOT=dir    render every theme to dir/<theme>.png and exit

mod boxdraw;
mod input;
mod renderer;
mod terminal;
mod theme;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersKeyState, ModifiersState};
use winit::window::{Window, WindowId};

use renderer::Renderer;
use terminal::{Terminal, UserEvent};
use theme::Theme;

/// How long snapshot mode lets the command run before capturing.
const SNAPSHOT_DELAY: Duration = Duration::from_millis(1500);

struct App {
    proxy: EventLoopProxy<UserEvent>,
    command: Option<String>,
    snapshot_dir: Option<PathBuf>,
    started: Instant,

    renderer: Option<Renderer>,
    terminal: Option<Terminal>,
    themes: Vec<Theme>,
    current: usize,
    mods: ModifiersState,
    /// Left Option acts as Meta; right Option still composes characters.
    option_as_meta: bool,
    /// Leftover trackpad scroll, in pixels, below one line.
    scroll_px: f64,
    mouse: Mouse,
}

#[derive(Default)]
struct Mouse {
    /// Pointer position in physical pixels.
    x: f64,
    y: f64,
    /// Dragging out a selection with the left button.
    selecting: bool,
    /// Button code held down while the program receives mouse events.
    reported_button: Option<u8>,
    /// Last cell reported to the program, to avoid repeating motion events.
    reported_cell: Option<(usize, usize)>,
    /// For double/triple click: when, where, and how many so far.
    last_click: Option<(Instant, (usize, usize), u8)>,
}

/// Clicks closer together than this count as a double/triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);

impl App {
    fn theme(&self) -> &Theme {
        &self.themes[self.current]
    }

    /// Recomputes the grid for the window and theme, and tells the shell.
    fn relayout(&mut self) {
        let theme = self.themes[self.current].clone();
        let (Some(renderer), Some(terminal)) = (self.renderer.as_mut(), self.terminal.as_mut())
        else {
            return;
        };
        terminal.resize(renderer.grid_size(&theme));
        renderer.window().request_redraw();
    }

    fn redraw(&self) {
        if let Some(r) = &self.renderer {
            r.window().request_redraw();
        }
    }

    /// App shortcuts (Cmd+...). Returns true if the key was handled.
    fn shortcut(&mut self, event: &KeyEvent) -> bool {
        if !self.mods.super_key() {
            return false;
        }
        let Key::Character(key) = &event.logical_key else {
            return false;
        };
        match key.as_str() {
            "t" => {
                self.current = (self.current + 1) % self.themes.len();
                self.relayout();
            }
            "=" | "+" => {
                if let Some(r) = self.renderer.as_mut() {
                    r.zoom_by(1.1);
                }
                self.relayout();
            }
            "-" => {
                if let Some(r) = self.renderer.as_mut() {
                    r.zoom_by(1.0 / 1.1);
                }
                self.relayout();
            }
            "c" => {
                if let Some(text) = self.terminal.as_ref().and_then(|t| t.selection_text()) {
                    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
                }
            }
            "v" => {
                let text = arboard::Clipboard::new().and_then(|mut c| c.get_text());
                if let (Ok(text), Some(t)) = (text, &self.terminal) {
                    t.paste(&text);
                }
            }
            _ => return false,
        }
        true
    }

    /// xterm modifier bits for mouse reports.
    fn mouse_mods(&self) -> u8 {
        4 * self.mods.shift_key() as u8
            + 8 * self.mods.alt_key() as u8
            + 16 * self.mods.control_key() as u8
    }

    fn pointer_cell(&self) -> Option<(usize, usize, bool)> {
        let (r, t) = (self.renderer.as_ref()?, self.terminal.as_ref()?);
        Some(r.cell_at(self.mouse.x, self.mouse.y, t.size()))
    }

    fn on_mouse_button(&mut self, state: ElementState, button: MouseButton) {
        let pressed = state == ElementState::Pressed;
        // macOS drags through its (transparent) native title bar.
        if pressed
            && !cfg!(target_os = "macos")
            && button == MouseButton::Left
            && let Some(r) = &self.renderer
            && self.mouse.y < renderer::TITLE_STRIP as f64 * r.window().scale_factor()
        {
            let _ = r.window().drag_window();
            return;
        }
        let Some((col, line, right_half)) = self.pointer_cell() else { return };
        let Some(t) = &self.terminal else { return };

        // Programs that asked for the mouse get it; Shift bypasses that so
        // you can still select text, as in other terminals.
        if t.mouse_mode() && !self.mods.shift_key() {
            let code = match button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                _ => return,
            } + self.mouse_mods();
            t.report_mouse(code, col, line, pressed, false);
            self.mouse.reported_button = pressed.then_some(code);
            self.mouse.reported_cell = Some((col, line));
            return;
        }

        if button != MouseButton::Left {
            return;
        }
        if !pressed {
            self.mouse.selecting = false;
            return;
        }
        let now = Instant::now();
        let clicks = match self.mouse.last_click {
            Some((at, cell, n)) if now - at < MULTI_CLICK && cell == (col, line) => n % 3 + 1,
            _ => 1,
        };
        self.mouse.last_click = Some((now, (col, line), clicks));
        let ty = match clicks {
            1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            _ => SelectionType::Lines,
        };
        t.start_selection(ty, col, line, right_half);
        self.mouse.selecting = true;
        self.redraw();
    }

    fn on_mouse_move(&mut self) {
        let Some((col, line, right_half)) = self.pointer_cell() else { return };
        let Some(t) = &self.terminal else { return };
        if self.mouse.selecting {
            t.update_selection(col, line, right_half);
            self.redraw();
            return;
        }
        if !t.mouse_mode() || self.mouse.reported_cell == Some((col, line)) {
            return;
        }
        self.mouse.reported_cell = Some((col, line));
        match self.mouse.reported_button {
            Some(code) => t.report_mouse(code, col, line, true, true),
            // Plain motion with no button held (only in any-motion mode).
            None if t.mode().contains(TermMode::MOUSE_MOTION) => {
                t.report_mouse(3 + self.mouse_mods(), col, line, true, true)
            }
            None => {}
        }
    }

    fn snapshot_themes(&mut self, dir: &std::path::Path) {
        std::fs::create_dir_all(dir).expect("create snapshot dir");
        for i in 0..self.themes.len() {
            self.current = i;
            self.relayout();
            let theme = self.themes[i].clone();
            let name = theme.name.to_lowercase().replace(' ', "-");
            if let (Some(r), Some(t)) = (self.renderer.as_mut(), &self.terminal) {
                let term = t.term.lock();
                r.snapshot(&theme, &term, &dir.join(format!("{name}.png")));
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("oneprompt")
            .with_inner_size(winit::dpi::LogicalSize::new(960.0, 600.0));
        // No visible title bar: the surface runs to the top edge. On macOS the
        // native title bar stays (transparent) so dragging, double-click zoom
        // and the window buttons keep working; elsewhere we drag ourselves.
        #[cfg(target_os = "macos")]
        let attrs = {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true)
        };
        #[cfg(not(target_os = "macos"))]
        let attrs = attrs.with_decorations(false);
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let mut renderer = Renderer::new(window);
        let size = renderer.grid_size(self.theme());
        match Terminal::spawn(self.proxy.clone(), size, self.command.clone()) {
            Ok(terminal) => self.terminal = Some(terminal),
            Err(err) => {
                eprintln!("oneprompt: could not start shell: {err}");
                event_loop.exit();
                return;
            }
        }
        renderer.window().request_redraw();
        self.renderer = Some(renderer);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Wakeup => self.redraw(),
            UserEvent::Title(title) => {
                if let Some(r) = &self.renderer {
                    r.window().set_title(&title);
                }
            }
            UserEvent::PtyWrite(text) => {
                if let Some(t) = &self.terminal {
                    t.write(text.into_bytes());
                }
            }
            // In snapshot mode the command may finish before we capture.
            UserEvent::Exit if self.snapshot_dir.is_some() => {}
            UserEvent::Exit => event_loop.exit(),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size.width, size.height);
                }
                self.relayout();
            }
            WindowEvent::ScaleFactorChanged { .. } => self.relayout(),
            WindowEvent::ModifiersChanged(mods) => {
                self.mods = mods.state();
                self.option_as_meta = !cfg!(target_os = "macos")
                    || mods.lalt_state() == ModifiersKeyState::Pressed;
            }
            WindowEvent::Focused(focused) => {
                if let Some(t) = &self.terminal {
                    t.set_focused(focused);
                }
                self.redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.shortcut(&event) {
                    return;
                }
                let Some(t) = &self.terminal else { return };
                let app_cursor = t.mode().contains(TermMode::APP_CURSOR);
                let bytes = input::key_to_bytes(&event, self.mods, self.option_as_meta, app_cursor);
                if let Some(bytes) = bytes {
                    t.clear_selection();
                    t.input(bytes);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                (self.mouse.x, self.mouse.y) = (position.x, position.y);
                self.on_mouse_move();
            }
            WindowEvent::MouseInput { state, button, .. } => self.on_mouse_button(state, button),
            WindowEvent::MouseWheel { delta, .. } => {
                let (Some(r), Some(t)) = (&self.renderer, &self.terminal) else { return };
                let line_px = r.cell_height() as f64;
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * 3.0) as i32,
                    MouseScrollDelta::PixelDelta(p) => {
                        self.scroll_px += p.y;
                        let lines = (self.scroll_px / line_px).trunc();
                        self.scroll_px -= lines * line_px;
                        lines as i32
                    }
                };
                if t.mouse_mode() && lines != 0 {
                    // Wheel buttons 64 (up) / 65 (down), one report per line.
                    if let Some((col, line, _)) = self.pointer_cell() {
                        let code = if lines > 0 { 64 } else { 65 } + self.mouse_mods();
                        for _ in 0..lines.unsigned_abs() {
                            t.report_mouse(code, col, line, true, false);
                        }
                    }
                } else {
                    t.scroll(lines);
                }
                self.redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Some(dir) = self.snapshot_dir.clone() {
                    if self.started.elapsed() >= SNAPSHOT_DELAY {
                        self.snapshot_themes(&dir);
                        event_loop.exit();
                    } else {
                        std::thread::sleep(Duration::from_millis(50));
                        self.redraw();
                    }
                    return;
                }
                let theme = self.themes[self.current].clone();
                if let (Some(r), Some(t)) = (self.renderer.as_mut(), &self.terminal) {
                    let term = t.term.lock();
                    r.render(&theme, &term);
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut command = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-e" => command = args.next(),
            "-h" | "--help" => {
                println!("usage: oneprompt [-e COMMAND]");
                return;
            }
            other => {
                eprintln!("oneprompt: unknown argument {other:?}");
                std::process::exit(2);
            }
        }
    }

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("create event loop");
    let mut app = App {
        proxy: event_loop.create_proxy(),
        command,
        snapshot_dir: std::env::var_os("ONEPROMPT_SNAPSHOT").map(PathBuf::from),
        started: Instant::now(),
        renderer: None,
        terminal: None,
        themes: theme::builtin(),
        current: 0,
        mods: ModifiersState::empty(),
        option_as_meta: !cfg!(target_os = "macos"),
        scroll_px: 0.0,
        mouse: Mouse::default(),
    };
    event_loop.run_app(&mut app).expect("run event loop");
}
