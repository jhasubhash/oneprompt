//! Shell integration: where each prompt is, and how each command ended.
//!
//! Shells mark their prompts with OSC 133 (the FinalTerm marks iTerm2,
//! kitty, WezTerm and Ghostty also read): `A` as a prompt starts, `B` where
//! it ends and typing begins, `C` as the typed command starts running, and
//! `D;<status>` when it is done. zsh gets them from a small `.zshenv` this
//! app puts first (`env`); fish 4 sends them itself; bash needs two lines
//! (README.md).
//!
//! Programs report progress the same way, with OSC 9;4 (`ESC ] 9 ; 4 ; state
//! ; percent`, as ConEmu, Windows Terminal and Ghostty read it): those are
//! taken out too, and sent as `UserEvent::Progress`.
//!
//! alacritty_terminal ignores these marks, and they only mean something
//! with their place in the grid, which is known once the terminal has
//! parsed the bytes before them. So the PTY is read through `Marked`, which
//! takes them out of the stream on the way in: a prompt becomes a hyperlink
//! (OSC 8) to `oneprompt-prompt:<n>`, so its cells carry the command's
//! number wherever they scroll, and the end of a command is sent to the UI
//! thread as `UserEvent::Command` with that number. Everything else passes
//! through untouched.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
use polling::{Event, PollMode, Poller};
use winit::event_loop::EventLoopProxy;

use crate::terminal::{PaneId, UserEvent};

/// The hyperlink address a prompt's cells carry: this, then the command's
/// number in its pane.
pub const PROMPT_URI: &str = "oneprompt-prompt:";

/// The number of the command typed at the prompt this cell belongs to.
pub fn prompt_number(uri: &str) -> Option<u64> {
    uri.strip_prefix(PROMPT_URI)?.parse().ok()
}

/// What a program says about its progress (OSC 9;4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    /// Done, or never mind: nothing to show.
    Clear,
    /// This share done, 0..1.
    Running(f32),
    /// Stopped with an error, at this share.
    Error(f32),
    /// Busy, with no idea how far along.
    Busy,
    /// Paused at this share.
    Paused(f32),
}

impl Progress {
    /// From OSC 9;4's state and percent.
    fn parse(text: &str) -> Option<Progress> {
        let mut parts = text.split(';');
        let state = parts.next()?.trim();
        let share = parts.next().and_then(|p| p.trim().parse::<f32>().ok()).unwrap_or(0.0).clamp(0.0, 100.0) / 100.0;
        Some(match state {
            "0" => Progress::Clear,
            "1" => Progress::Running(share),
            "2" => Progress::Error(share),
            "3" => Progress::Busy,
            "4" => Progress::Paused(share),
            _ => return None,
        })
    }
}

/// How a command ended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Finished {
    pub status: i32,
    /// From the command starting (`C`) to it ending (`D`).
    pub seconds: f32,
}

impl Finished {
    /// Failed: a non-zero status, except a command stopped with Ctrl+C
    /// (130), which was meant.
    pub fn failed(&self) -> bool {
        self.status != 0 && self.status != 130
    }
}

/// The PTY, read through the mark scanner.
pub struct Marked {
    pty: Pty,
    reader: Reader,
}

impl Marked {
    pub fn new(pty: Pty, pane: PaneId, proxy: EventLoopProxy<UserEvent>) -> io::Result<Self> {
        // A second handle on the same open file: the event loop polls the
        // PTY's own and reads through this one.
        let file = pty.file().try_clone()?;
        let progress = proxy.clone();
        let mut reader = Reader::new(file, Box::new(move |n, finished| {
            let _ = proxy.send_event(UserEvent::Command(pane, n, finished));
        }));
        reader.progress = Some(Box::new(move |p| {
            let _ = progress.send_event(UserEvent::Progress(pane, p));
        }));
        Ok(Self { pty, reader })
    }
}

impl EventedReadWrite for Marked {
    type Reader = Reader;
    type Writer = File;

    unsafe fn register(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        unsafe { self.pty.register(poll, interest, mode) }
    }

    fn reregister(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        self.pty.reregister(poll, interest, mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.pty.deregister(poll)
    }

    fn reader(&mut self) -> &mut Reader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut File {
        self.pty.writer()
    }
}

impl EventedPty for Marked {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.pty.next_child_event()
    }
}

impl OnResize for Marked {
    fn on_resize(&mut self, size: WindowSize) {
        self.pty.on_resize(size);
    }
}

/// Where a mark starts: a prompt mark, or a progress report.
const MARK: &[u8] = b"\x1b]133;";
const PROGRESS: &[u8] = b"\x1b]9;4;";
/// A mark longer than this is not one; it passes through as it is.
const MAX_MARK: usize = 256;

/// Reads the shell's output, taking the marks out (see the module docs).
pub struct Reader {
    source: Box<dyn Read + Send>,
    /// Bytes ready to hand on, from `at`.
    out: Vec<u8>,
    at: usize,
    /// The start of a mark split across reads, kept until the rest comes.
    carry: Vec<u8>,
    raw: Vec<u8>,
    /// The current prompt's number, and when its command started.
    prompt: u64,
    started: Option<Instant>,
    done: Box<dyn FnMut(u64, Finished) + Send>,
    progress: Option<Box<dyn FnMut(Progress) + Send>>,
}

impl Reader {
    fn new(source: impl Read + Send + 'static, done: Box<dyn FnMut(u64, Finished) + Send>) -> Self {
        Self { source: Box::new(source), out: Vec::new(), at: 0, carry: Vec::new(), raw: vec![0; 0x10000], prompt: 0, started: None, done, progress: None }
    }

    /// Takes the marks out of `bytes` (after any carried over) into `out`.
    fn scan(&mut self, bytes: &[u8]) {
        let mut input = std::mem::take(&mut self.carry);
        input.extend_from_slice(bytes);
        let mut from = 0;
        let mut i = 0;
        while let Some(j) = input[i..].iter().position(|&b| b == 0x1b).map(|j| i + j) {
            let rest = &input[j..];
            // Possibly the start of one, cut off by the end of the read.
            if rest.len() < PROGRESS.len().max(MARK.len()) && (MARK.starts_with(rest) || PROGRESS.starts_with(rest)) {
                self.out.extend_from_slice(&input[from..j]);
                self.carry = rest.to_vec();
                return;
            }
            let prefix = if rest.starts_with(MARK) {
                MARK
            } else if rest.starts_with(PROGRESS) {
                PROGRESS
            } else {
                i = j + 1;
                continue;
            };
            let body = &rest[prefix.len()..];
            let end = body.iter().enumerate().find_map(|(k, &b)| match b {
                0x07 => Some((k, 1)),
                0x1b if body.get(k + 1) == Some(&b'\\') => Some((k, 2)),
                _ => None,
            });
            let Some((len, terminator)) = end else {
                if rest.len() < MAX_MARK {
                    self.out.extend_from_slice(&input[from..j]);
                    self.carry = rest.to_vec();
                    return;
                }
                i = j + 1;
                continue;
            };
            self.out.extend_from_slice(&input[from..j]);
            let body = body[..len].to_vec();
            if prefix == MARK {
                self.mark(&body);
            } else if let (Some(report), Some(p)) = (self.progress.as_mut(), Progress::parse(&String::from_utf8_lossy(&body))) {
                report(p);
            }
            from = j + prefix.len() + len + terminator;
            i = from;
        }
        self.out.extend_from_slice(&input[from..]);
    }

    /// One mark's text, after `133;`.
    fn mark(&mut self, body: &[u8]) {
        let text = String::from_utf8_lossy(body);
        let mut parts = text.split(';');
        match parts.next().unwrap_or("") {
            "A" => {
                self.prompt += 1;
                self.started = None;
                let n = self.prompt;
                self.out.extend_from_slice(format!("\x1b]8;id=oneprompt-{n};{PROMPT_URI}{n}\x1b\\").as_bytes());
            }
            "C" => {
                self.out.extend_from_slice(b"\x1b]8;;\x1b\\");
                self.started = Some(Instant::now());
            }
            "D" => {
                self.out.extend_from_slice(b"\x1b]8;;\x1b\\");
                // An empty line has no command to report on.
                if let Some(started) = self.started.take() {
                    let status = parts.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
                    (self.done)(self.prompt, Finished { status, seconds: started.elapsed().as_secs_f32() });
                }
            }
            // B (the prompt's end) needs nothing: the link runs on through
            // the typed command to C.
            _ => {}
        }
    }
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.at >= self.out.len() {
            self.out.clear();
            self.at = 0;
            let raw = std::mem::take(&mut self.raw);
            let mut raw = raw;
            let got = match self.source.read(&mut raw) {
                Ok(got) => got,
                // Nothing more for now: a bare start of a sequence held back
                // in case it was a mark is handed on after all.
                Err(err) if err.kind() == io::ErrorKind::WouldBlock && !self.carry.is_empty() && self.carry.len() < PROGRESS.len().max(MARK.len()) => {
                    self.raw = raw;
                    self.out = std::mem::take(&mut self.carry);
                    break;
                }
                Err(err) => {
                    self.raw = raw;
                    return Err(err);
                }
            };
            if got == 0 {
                self.raw = raw;
                self.out = std::mem::take(&mut self.carry);
                if self.out.is_empty() {
                    return Ok(0);
                }
                break;
            }
            self.scan(&raw[..got]);
            self.raw = raw;
        }
        let n = buf.len().min(self.out.len() - self.at);
        buf[..n].copy_from_slice(&self.out[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// The environment that makes the user's shell mark its prompts, for a
/// login shell (not `-e command`). zsh reads its startup files from
/// `ZDOTDIR`: it is pointed at a folder whose `.zshenv` puts the user's
/// own `ZDOTDIR` back, reads their `.zshenv` and adds the marks, so their
/// other files are read as always. Other shells are left as they are.
pub fn env() -> HashMap<String, String> {
    let mut env = HashMap::new();
    let shell = std::env::var("SHELL").unwrap_or_default();
    if !shell.ends_with("/zsh") {
        return env;
    }
    let Some(dir) = zsh_dir() else { return env };
    if let Ok(own) = std::env::var("ZDOTDIR") {
        env.insert("ONEPROMPT_ZDOTDIR".into(), own);
    }
    env.insert("ZDOTDIR".into(), dir.to_string_lossy().into_owned());
    env
}

const ZSHENV: &str = include_str!("shell/zshenv");

/// The folder with the integration's `.zshenv`, written if it is missing
/// or from another version.
fn zsh_dir() -> Option<PathBuf> {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let dir = std::env::temp_dir().join(format!("oneprompt-{user}")).join("zsh");
    let file = dir.join(".zshenv");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(ZSHENV) {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&file, ZSHENV).ok()?;
    }
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Reads `chunks` (one per read) through a reader; returns what came
    /// out and the commands it reported.
    fn run(chunks: &[&[u8]]) -> (Vec<u8>, Vec<(u64, i32)>) {
        struct Chunks(Vec<Vec<u8>>);
        impl Read for Chunks {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                let chunk = self.0.remove(0);
                buf[..chunk.len()].copy_from_slice(&chunk);
                Ok(chunk.len())
            }
        }
        let reported = Arc::new(Mutex::new(Vec::new()));
        let seen = reported.clone();
        let source = Chunks(chunks.iter().map(|c| c.to_vec()).collect());
        let mut reader = Reader::new(source, Box::new(move |n, f| seen.lock().unwrap().push((n, f.status))));
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        let reported = reported.lock().unwrap().clone();
        (out, reported)
    }

    #[test]
    fn prompts_become_links_and_ends_are_reported() {
        let (out, done) = run(&[b"\x1b]133;D;0\x07\x1b]133;A\x07$ ls\x1b]133;C\x07files\r\n\x1b]133;D;2\x07\x1b]133;A\x07$ "]);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text,
            "\x1b]8;;\x1b\\\x1b]8;id=oneprompt-1;oneprompt-prompt:1\x1b\\$ ls\x1b]8;;\x1b\\files\r\n\x1b]8;;\x1b\\\x1b]8;id=oneprompt-2;oneprompt-prompt:2\x1b\\$ "
        );
        // The first D (before any command) and an empty line report nothing.
        assert_eq!(done, vec![(1, 2)]);
        assert_eq!(prompt_number("oneprompt-prompt:12"), Some(12));
        assert_eq!(prompt_number("https://a.io"), None);
    }

    #[test]
    fn marks_split_across_reads_still_count_and_other_sequences_pass() {
        let (out, done) = run(&[b"\x1b]133;A\x07x\x1b]1", b"33;C\x1b\\\x1b[31mred\x1b", b"]133;D;1\x07end"]);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\x1b[31mred"), "{text:?}");
        assert!(text.ends_with("end"));
        assert!(!text.contains("133;"));
        assert_eq!(done, vec![(1, 1)]);
        let (out, _) = run(&[b"plain \x1b[0m text"]);
        assert_eq!(out, b"plain \x1b[0m text");
    }

    #[test]
    fn progress_reports_are_taken_out() {
        assert_eq!(Progress::parse("1;40"), Some(Progress::Running(0.4)));
        assert_eq!(Progress::parse("2;250"), Some(Progress::Error(1.0)));
        assert_eq!(Progress::parse("3"), Some(Progress::Busy));
        assert_eq!(Progress::parse("0;0"), Some(Progress::Clear));
        assert_eq!(Progress::parse("9;1"), None);
        let (out, _) = run(&[b"a\x1b]9;4;1;5", b"0\x1b\\b\x1b]9;1;notify\x07"]);
        // The progress report goes; another OSC 9 (a notification) stays.
        assert_eq!(out, b"ab\x1b]9;1;notify\x07");
    }

    #[test]
    fn interrupted_commands_are_not_failures() {
        assert!(Finished { status: 1, seconds: 0.0 }.failed());
        assert!(!Finished { status: 130, seconds: 0.0 }.failed());
        assert!(!Finished { status: 0, seconds: 0.0 }.failed());
    }
}
