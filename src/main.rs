//! oneprompt: a terminal where the background and the type are part of the design.
//!
//! This first milestone is a rendering prototype: a window with procedurally
//! textured themes and per-theme fonts, plus a local line editor to see how
//! typing feels. The shell (PTY) and VT parsing come next; see README.md.

mod renderer;
mod theme;

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use renderer::Renderer;
use theme::Theme;

struct App {
    renderer: Option<Renderer>,
    themes: Vec<Theme>,
    current: usize,
    /// Lines already "submitted" with Enter.
    history: Vec<String>,
    /// The line being typed.
    input: String,
}

impl App {
    fn new() -> Self {
        Self {
            renderer: None,
            themes: theme::builtin(),
            current: 0,
            history: Vec::new(),
            input: String::new(),
        }
    }

    fn theme(&self) -> &Theme {
        &self.themes[self.current]
    }

    fn content(&self) -> String {
        let mut out = format!(
            "oneprompt  ·  theme: {}  ·  F1 switches theme\n\n",
            self.theme().name
        );
        for line in &self.history {
            out.push_str("~ ❯ ");
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("~ ❯ ");
        out.push_str(&self.input);
        out.push('▏');
        out
    }

    fn refresh(&mut self) {
        let content = self.content();
        let theme = self.themes[self.current].clone();
        if let Some(r) = self.renderer.as_mut() {
            r.set_text(&theme, &content);
            r.window().request_redraw();
        }
    }

    /// Writes one PNG per built-in theme into `dir`, with some sample input.
    fn snapshot_themes(&mut self, dir: &std::path::Path) {
        std::fs::create_dir_all(dir).expect("create snapshot dir");
        self.history = vec!["ls ~/notes".into(), "cat letter.txt".into()];
        self.input = "echo \"writing with ink on paper\"".into();
        for i in 0..self.themes.len() {
            self.current = i;
            let content = self.content();
            let theme = self.themes[i].clone();
            let name = theme.name.to_lowercase().replace(' ', "-");
            if let Some(r) = self.renderer.as_mut() {
                r.set_text(&theme, &content);
                r.snapshot(&theme, &dir.join(format!("{name}.png")));
            }
        }
    }

    fn on_key(&mut self, event: &KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::F1) => {
                self.current = (self.current + 1) % self.themes.len();
            }
            Key::Named(NamedKey::Enter) => {
                self.history.push(std::mem::take(&mut self.input));
            }
            Key::Named(NamedKey::Backspace) => {
                self.input.pop();
            }
            _ => match &event.text {
                Some(text) if !text.chars().any(char::is_control) => self.input.push_str(text),
                _ => return,
            },
        }
        self.refresh();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("oneprompt")
            .with_inner_size(winit::dpi::LogicalSize::new(960.0, 600.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        self.renderer = Some(Renderer::new(window));
        self.refresh();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size.width, size.height);
                }
                self.refresh();
            }
            WindowEvent::ScaleFactorChanged { .. } => self.refresh(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.on_key(&event);
            }
            WindowEvent::RedrawRequested => {
                if let Some(dir) = std::env::var_os("ONEPROMPT_SNAPSHOT") {
                    self.snapshot_themes(std::path::Path::new(&dir));
                    event_loop.exit();
                    return;
                }
                let theme = self.themes[self.current].clone();
                if let Some(r) = self.renderer.as_mut() {
                    r.render(&theme);
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("create event loop");
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("run event loop");
}
