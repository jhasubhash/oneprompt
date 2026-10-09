//! The shell side: a PTY running the user's shell, parsed into a grid by
//! alacritty_terminal. Its reader thread wakes the winit loop via
//! `UserEvent`s; the renderer locks the grid when drawing.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use winit::event_loop::EventLoopProxy;

/// Events from the PTY thread to the UI thread.
#[derive(Debug)]
pub enum UserEvent {
    /// New output was parsed; redraw.
    Wakeup,
    Title(String),
    /// The terminal wants to answer a query (e.g. cursor position).
    PtyWrite(String),
    /// The shell exited.
    Exit,
}

#[derive(Clone)]
pub struct Listener(EventLoopProxy<UserEvent>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let event = match event {
            Event::Wakeup => UserEvent::Wakeup,
            Event::Title(title) => UserEvent::Title(title),
            Event::ResetTitle => UserEvent::Title("oneprompt".into()),
            Event::PtyWrite(text) => UserEvent::PtyWrite(text),
            Event::Exit | Event::ChildExit(_) => UserEvent::Exit,
            _ => return,
        };
        let _ = self.0.send_event(event);
    }
}

/// Grid size in cells plus the pixel size of one cell (the PTY reports both).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSize {
    pub columns: usize,
    pub lines: usize,
    pub cell_width: f32,
    pub cell_height: f32,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

impl GridSize {
    fn window_size(&self) -> WindowSize {
        WindowSize {
            num_lines: self.lines as u16,
            num_cols: self.columns as u16,
            cell_width: self.cell_width as u16,
            cell_height: self.cell_height as u16,
        }
    }
}

pub struct Terminal {
    pub term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    size: GridSize,
}

impl Terminal {
    /// Spawns the user's login shell, or `/bin/sh -c command` when given.
    pub fn spawn(
        proxy: EventLoopProxy<UserEvent>,
        size: GridSize,
        command: Option<String>,
    ) -> std::io::Result<Self> {
        tty::setup_env();
        let options = tty::Options {
            shell: command.map(|c| tty::Shell::new("/bin/sh".into(), vec!["-c".into(), c])),
            working_directory: std::env::var_os("HOME").map(Into::into),
            drain_on_exit: true,
            env: HashMap::new(),
        };
        let pty = tty::new(&options, size.window_size(), 0)?;
        let listener = Listener(proxy);
        let term = Arc::new(FairMutex::new(Term::new(
            Config::default(),
            &size,
            listener.clone(),
        )));
        let event_loop = EventLoop::new(term.clone(), listener, pty, options.drain_on_exit, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok(Self { term, sender, size })
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.sender.send(Msg::Input(bytes.into()));
    }

    /// Sends typed input, snapping the view back to the live screen first.
    pub fn input(&self, bytes: Vec<u8>) {
        self.term.lock().scroll_display(Scroll::Bottom);
        self.write(bytes);
    }

    pub fn paste(&self, text: &str) {
        // Normalise newlines to carriage returns, as a keyboard would send.
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.mode().contains(TermMode::BRACKETED_PASTE) {
            self.input(format!("\x1b[200~{text}\x1b[201~").into_bytes());
        } else {
            self.input(text.into_bytes());
        }
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    /// Scrolls history by `lines` (positive = up). Full-screen apps such as
    /// less or vim get arrow keys instead, like other terminals do.
    pub fn scroll(&self, lines: i32) {
        if lines == 0 {
            return;
        }
        let mode = self.mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let key: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            self.write(key.repeat(lines.unsigned_abs() as usize));
        } else {
            self.term.lock().scroll_display(Scroll::Delta(lines));
        }
    }

    pub fn resize(&mut self, size: GridSize) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.term.lock().resize(size);
        let _ = self.sender.send(Msg::Resize(size.window_size()));
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}
