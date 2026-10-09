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

mod input;
mod renderer;
mod terminal;
mod theme;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState};
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
    /// Leftover trackpad scroll, in pixels, below one line.
    scroll_px: f64,
}

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
            WindowEvent::ModifiersChanged(mods) => self.mods = mods.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.shortcut(&event) {
                    return;
                }
                let Some(t) = &self.terminal else { return };
                let app_cursor = t.mode().contains(TermMode::APP_CURSOR);
                if let Some(bytes) = input::key_to_bytes(&event, self.mods, app_cursor) {
                    t.input(bytes);
                }
            }
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
                t.scroll(lines);
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
        scroll_px: 0.0,
    };
    event_loop.run_app(&mut app).expect("run event loop");
}
