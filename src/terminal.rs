//! The shell side: a PTY running the user's shell, parsed into a grid by
//! alacritty_terminal. Its reader thread wakes the winit loop via
//! `UserEvent`s; the renderer locks the grid when drawing.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{ClearMode, Handler, Rgb};
use winit::event_loop::EventLoopProxy;

/// Answers a colour query: formats the reply for a given colour.
pub type ColorReply = Arc<dyn Fn(Rgb) -> String + Sync + Send + 'static>;

/// Which pane a terminal event came from (each pane runs its own shell).
pub type PaneId = u64;

/// Events from the PTY threads to the UI thread.
pub enum UserEvent {
    /// New output was parsed; redraw.
    Wakeup(PaneId),
    /// `None`: back to the default title.
    Title(PaneId, Option<String>),
    /// The terminal wants to answer a query (e.g. cursor position).
    PtyWrite(PaneId, String),
    /// The shell exited.
    Exit(PaneId),
    /// The settings file was saved (or created / deleted).
    ConfigChanged,
    /// A program asked for a colour (OSC 4/10/11/12): palette index, or
    /// 256/257/258 for foreground, background, cursor.
    ColorRequest(PaneId, usize, ColorReply),
    /// An item was chosen in the right-click menu.
    Menu(crate::menu::Action),
    /// A command finished (shell integration): the pane, the number of the
    /// prompt it was typed at, and how it ended.
    Command(PaneId, u64, crate::marks::Finished),
    /// The shell cleared the screen (`clear`, Ctrl+L), outside full-screen
    /// apps.
    /// Answer on the channel once the frame on show has been kept.
    Cleared(PaneId, std::sync::mpsc::SyncSender<()>),
    /// A program showed a picture (images.rs).
    Picture(PaneId, Arc<crate::images::Picture>),
    /// A program reported its progress (OSC 9;4).
    Progress(PaneId, crate::marks::Progress),
}

#[derive(Clone)]
pub struct Listener(EventLoopProxy<UserEvent>, PaneId);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let id = self.1;
        let event = match event {
            Event::Wakeup => UserEvent::Wakeup(id),
            Event::Title(title) => UserEvent::Title(id, Some(title)),
            Event::ResetTitle => UserEvent::Title(id, None),
            Event::PtyWrite(text) => UserEvent::PtyWrite(id, text),
            Event::ColorRequest(index, reply) => UserEvent::ColorRequest(id, index, reply),
            Event::Exit | Event::ChildExit(_) => UserEvent::Exit(id),
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
    /// The size again, for the PTY reader to fit pictures to.
    shared_size: Arc<std::sync::Mutex<GridSize>>,
    /// A handle on the PTY, to ask which process is in the foreground (the
    /// shell at its prompt, or the command it runs) and follow its working
    /// directory. The PTY's child is `login`, not the shell, on macOS.
    pub pty: Option<std::fs::File>,
}

impl Terminal {
    /// Spawns the user's login shell, or `/bin/sh -c command` when given.
    /// `integrate`: have the login shell mark its prompts (marks.rs).
    pub fn spawn(
        proxy: EventLoopProxy<UserEvent>,
        id: PaneId,
        size: GridSize,
        command: Option<String>,
        cwd: Option<std::path::PathBuf>,
        integrate: bool,
    ) -> std::io::Result<Self> {
        tty::setup_env();
        let mut env = if integrate && command.is_none() { crate::marks::env() } else { HashMap::new() };
        // Programs in the terminal can find the app: the shell's md and
        // imgcat run it.
        if let Ok(exe) = std::env::current_exe() {
            env.insert("ONEPROMPT_EXE".into(), exe.to_string_lossy().into_owned());
        }
        let options = tty::Options {
            shell: command.map(|c| tty::Shell::new("/bin/sh".into(), vec!["-c".into(), c])),
            working_directory: cwd.or_else(|| std::env::var_os("HOME").map(Into::into)),
            drain_on_exit: true,
            env,
        };
        let pty = tty::new(&options, size.window_size(), 0)?;
        let pty_handle = pty.file().try_clone().ok();
        let shared_size = Arc::new(std::sync::Mutex::new(size));
        let pty = crate::marks::Marked::new(pty, id, proxy.clone(), shared_size.clone())?;
        let listener = Listener(proxy, id);
        let term = Arc::new(FairMutex::new(Term::new(
            Config::default(),
            &size,
            listener.clone(),
        )));
        let event_loop = EventLoop::new(term.clone(), listener, pty, options.drain_on_exit, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok(Self { term, sender, size, shared_size, pty: pty_handle })
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

    pub fn size(&self) -> GridSize {
        self.size
    }

    pub fn set_focused(&self, focused: bool) {
        let mut term = self.term.lock();
        term.is_focused = focused;
        if term.mode().contains(TermMode::FOCUS_IN_OUT) {
            drop(term);
            self.write(if focused { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
        }
    }

    /// Converts a viewport cell to a grid point (accounts for scrollback).
    fn grid_point(term: &Term<Listener>, col: usize, line: usize) -> Point {
        let offset = term.grid().display_offset() as i32;
        Point::new(Line(line as i32 - offset), Column(col))
    }

    pub fn start_selection(&self, ty: SelectionType, col: usize, line: usize, right_half: bool) {
        let mut term = self.term.lock();
        let point = Self::grid_point(&term, col, line);
        let side = if right_half { Side::Right } else { Side::Left };
        term.selection = Some(Selection::new(ty, point, side));
    }

    pub fn update_selection(&self, col: usize, line: usize, right_half: bool) {
        let mut term = self.term.lock();
        let point = Self::grid_point(&term, col, line);
        let side = if right_half { Side::Right } else { Side::Left };
        if let Some(selection) = term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    /// Selects everything: the scrollback and the screen.
    pub fn select_all(&self) {
        let mut term = self.term.lock();
        let (top, bottom) = (term.grid().topmost_line(), term.grid().bottommost_line());
        let last = term.grid().last_column();
        let mut selection = Selection::new(SelectionType::Lines, Point::new(top, Column(0)), Side::Left);
        selection.update(Point::new(bottom, last), Side::Right);
        term.selection = Some(selection);
    }

    /// Clears the screen and the scrollback (Cmd+K). The shell is sent
    /// Ctrl+L so it draws its prompt again; full-screen apps keep their
    /// screen and only lose the scrollback.
    pub fn clear(&self) {
        let mut term = self.term.lock();
        term.selection = None;
        term.scroll_display(Scroll::Bottom);
        if term.mode().contains(TermMode::ALT_SCREEN) {
            term.clear_screen(ClearMode::Saved);
            return;
        }
        term.clear_screen(ClearMode::All);
        term.clear_screen(ClearMode::Saved);
        drop(term);
        self.write(&b"\x0c"[..]);
    }

    /// Forgets a selection that covers nothing (a click without a drag).
    pub fn drop_empty_selection(&self) {
        let mut term = self.term.lock();
        if term.selection.as_ref().is_some_and(|s| s.is_empty()) {
            term.selection = None;
        }
    }

    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }

    pub fn selection_text(&self) -> Option<String> {
        let term = self.term.lock();
        if term.selection.as_ref().is_some_and(|s| s.is_empty()) {
            return None;
        }
        term.selection_to_string().filter(|s| !s.is_empty())
    }

    /// Whether the running program wants mouse events (vim, htop, tmux, ...).
    pub fn mouse_mode(&self) -> bool {
        self.mode().intersects(TermMode::MOUSE_MODE)
    }

    /// Reports a mouse event to the program. `button` is the xterm button
    /// code (0 left, 1 middle, 2 right, 64/65 wheel) with modifier bits added,
    /// `motion` marks drag/move reports.
    pub fn report_mouse(&self, button: u8, col: usize, line: usize, pressed: bool, motion: bool) {
        if let Some(bytes) = encode_mouse(self.mode(), button, col, line, pressed, motion) {
            self.write(bytes);
        }
    }

    pub fn resize(&mut self, size: GridSize) {
        if size == self.size {
            return;
        }
        self.size = size;
        if let Ok(mut shared) = self.shared_size.lock() {
            *shared = size;
        }
        self.term.lock().resize(size);
        let _ = self.sender.send(Msg::Resize(size.window_size()));
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}

/// xterm mouse report for `mode`, or `None` if the program did not ask for
/// this kind of event.
fn encode_mouse(
    mode: TermMode,
    button: u8,
    col: usize,
    line: usize,
    pressed: bool,
    motion: bool,
) -> Option<Vec<u8>> {
    if motion && !mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
        return None;
    }
    let code = button + if motion { 32 } else { 0 };
    let (x, y) = (col + 1, line + 1);
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if pressed { 'M' } else { 'm' };
        Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes())
    } else {
        // Legacy encoding: releases are button 3, and coordinates are single
        // bytes offset by 32, so they cap at 223.
        let code = if pressed { code } else { 3 + (code & !0b11) };
        let byte = |v: usize| (32 + v.min(223)) as u8;
        Some(vec![0x1b, b'[', b'M', 32 + code, byte(x), byte(y)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_press_and_release() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(encode_mouse(mode, 0, 4, 9, true, false).unwrap(), b"\x1b[<0;5;10M");
        assert_eq!(encode_mouse(mode, 0, 4, 9, false, false).unwrap(), b"\x1b[<0;5;10m");
    }

    #[test]
    fn legacy_release_is_button_three() {
        let mode = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(encode_mouse(mode, 2, 0, 0, true, false).unwrap(), vec![0x1b, b'[', b'M', 34, 33, 33]);
        assert_eq!(encode_mouse(mode, 2, 0, 0, false, false).unwrap(), vec![0x1b, b'[', b'M', 35, 33, 33]);
    }

    #[test]
    fn motion_needs_drag_or_motion_mode() {
        let click_only = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert!(encode_mouse(click_only, 0, 1, 1, true, true).is_none());
        let drag = TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE;
        assert_eq!(encode_mouse(drag, 0, 1, 1, true, true).unwrap(), b"\x1b[<32;2;2M");
    }
}
