//! The file browser (Cmd+B, or File Browser in the menu): a column down the
//! right of a pane listing the folder its shell is in, following the shell
//! as it moves. Folders come first, then files, each marked by kind in the
//! theme's inks. The keyboard or the pointer moves through it; typing
//! filters it. Opening a file shows it in the terminal: Markdown rendered
//! (`oneprompt md`), pictures inline (`oneprompt img`), other text in the
//! pager. The rest go to their app. Paths can be copied, put at the prompt
//! or dragged onto it, the shell sent into a folder, and files made,
//! renamed or moved to the Trash.
//!
//! This file is the browser's state and what its actions do to the file
//! system; the renderer draws it (`Renderer::set_browsers`) and main.rs
//! routes keys, clicks and menu items to it.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

/// How often the folder and the shell's place are looked at again.
pub const RECHECK_SECS: f32 = 0.8;

/// Files longer than this are never sniffed for text past this many bytes.
const SNIFF: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
    /// A link: to a folder (true) or to a file or nothing.
    Link(bool),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub executable: bool,
}

impl Entry {
    pub fn is_folder(&self) -> bool {
        matches!(self.kind, Kind::Folder | Kind::Link(true))
    }

    pub fn hidden(&self) -> bool {
        self.name.starts_with('.')
    }
}

/// How a file opens in the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opener {
    Markdown,
    Picture,
    Text,
    /// Not something the terminal shows: its own app.
    App,
}

/// A name being typed in the browser: a new folder or file, or a new name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Naming {
    NewFolder(String),
    NewFile(String),
    Rename(PathBuf, String),
}

impl Naming {
    pub fn text(&self) -> &str {
        match self {
            Naming::NewFolder(t) | Naming::NewFile(t) | Naming::Rename(_, t) => t,
        }
    }

    pub fn text_mut(&mut self) -> &mut String {
        match self {
            Naming::NewFolder(t) | Naming::NewFile(t) | Naming::Rename(_, t) => t,
        }
    }

    pub fn prompt(&self) -> &'static str {
        match self {
            Naming::NewFolder(_) => "New folder",
            Naming::NewFile(_) => "New file",
            Naming::Rename(..) => "Rename to",
        }
    }
}

pub struct Browser {
    /// The folder shown.
    pub dir: PathBuf,
    /// Everything in it, folders first, then by name.
    entries: Vec<Entry>,
    /// The entries shown (hidden ones and the filter left out), as indices
    /// into `entries`.
    shown: Vec<usize>,
    /// The entry chosen, as an index into `shown`.
    pub selected: usize,
    /// Typed to narrow the list down: names containing it, any case.
    pub filter: String,
    pub show_hidden: bool,
    /// A name being typed, instead of the filter.
    pub naming: Option<Naming>,
    /// Why the folder could not be read.
    pub error: Option<String>,
    /// It has the keyboard (else the terminal beside it has).
    pub keys: bool,
    /// The shell's folder when last looked at: when it moves, the browser
    /// follows.
    pub shell_dir: Option<PathBuf>,
    /// The folder's modification time when read, to see it change.
    stamp: Option<SystemTime>,
    pub checked: Instant,
    /// Asked to move this to the Trash once already: the second time does.
    pub trash_armed: Option<PathBuf>,
    /// The folders gone through to get here, each with the entry chosen
    /// in it, for Esc to step back through.
    history: Vec<(PathBuf, Option<String>)>,
    /// When it began to come in, and to go out again: it comes and goes
    /// the theme's way, as the settings panel does.
    pub opened: Instant,
    pub closing: Option<Instant>,
}

impl Browser {
    pub fn new(dir: PathBuf) -> Self {
        let mut b = Self {
            dir: dir.clone(),
            entries: Vec::new(),
            shown: Vec::new(),
            selected: 0,
            filter: String::new(),
            show_hidden: false,
            naming: None,
            error: None,
            keys: true,
            shell_dir: Some(dir),
            stamp: None,
            checked: Instant::now(),
            trash_armed: None,
            history: Vec::new(),
            opened: Instant::now(),
            closing: None,
        };
        b.reload();
        b
    }

    /// The entries shown, in order.
    pub fn shown(&self) -> impl ExactSizeIterator<Item = &Entry> {
        self.shown.iter().map(|&i| &self.entries[i])
    }

    pub fn len(&self) -> usize {
        self.shown.len()
    }

    /// The chosen entry.
    pub fn current(&self) -> Option<&Entry> {
        self.shown.get(self.selected).map(|&i| &self.entries[i])
    }

    /// How many entries the folder has in all, and how many are hidden.
    pub fn counts(&self) -> (usize, usize) {
        (self.entries.len(), self.entries.iter().filter(|e| e.hidden()).count())
    }

    /// Reads the folder again, keeping the chosen entry chosen.
    pub fn reload(&mut self) {
        let keep = self.current().map(|e| e.name.clone());
        self.stamp = fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
        match read_dir(&self.dir) {
            Ok(entries) => {
                self.entries = entries;
                self.error = None;
            }
            Err(err) => {
                self.entries.clear();
                self.error = Some(err.to_string());
            }
        }
        self.refilter();
        if let Some(name) = keep {
            self.select_name(&name);
        }
    }

    /// Reads the folder again if it has changed since it was read.
    pub fn refresh(&mut self) -> bool {
        let stamp = fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
        if stamp != self.stamp {
            self.reload();
            return true;
        }
        false
    }

    /// Shows `dir`, with `select` chosen if it is in it.
    pub fn go(&mut self, dir: PathBuf, select: Option<&str>) {
        self.dir = dir;
        self.filter.clear();
        self.naming = None;
        self.selected = 0;
        self.trash_armed = None;
        self.reload();
        if let Some(name) = select {
            self.select_name(name);
        }
    }

    /// Goes to `dir` as the user asked, so Esc can come back here.
    pub fn navigate(&mut self, dir: PathBuf, select: Option<&str>) {
        if dir == self.dir {
            return;
        }
        let chosen = self.current().map(|e| e.name.clone());
        self.history.push((self.dir.clone(), chosen));
        self.go(dir, select);
    }

    /// Goes where the shell went: the way back starts over from there.
    pub fn follow(&mut self, dir: PathBuf, select: Option<&str>) {
        self.history.clear();
        self.go(dir, select);
    }

    /// Back to the folder before this one, with what was chosen there;
    /// false when this is where the browser started.
    pub fn back(&mut self) -> bool {
        let Some((dir, chosen)) = self.history.pop() else { return false };
        self.go(dir, chosen.as_deref());
        true
    }

    /// Whether there is a folder to go back to.
    pub fn can_go_back(&self) -> bool {
        !self.history.is_empty()
    }

    /// Up to the folder this one is in, with this one chosen there.
    pub fn up(&mut self) {
        let Some(parent) = self.dir.parent().map(Path::to_path_buf) else { return };
        let name = self.dir.file_name().map(|n| n.to_string_lossy().into_owned());
        self.navigate(parent, name.as_deref());
    }

    /// Into the chosen folder; false if a file is chosen.
    pub fn enter(&mut self) -> bool {
        match self.current() {
            Some(e) if e.is_folder() => {
                let path = e.path.clone();
                self.navigate(path, None);
                true
            }
            _ => false,
        }
    }

    pub fn select_name(&mut self, name: &str) {
        if let Some(i) = self.shown.iter().position(|&i| self.entries[i].name == name) {
            self.selected = i;
        }
    }

    /// Moves the choice by `by` entries, staying in the list.
    pub fn move_by(&mut self, by: isize) {
        if self.shown.is_empty() {
            return;
        }
        self.selected = (self.selected as isize + by).clamp(0, self.shown.len() as isize - 1) as usize;
        self.trash_armed = None;
    }

    pub fn select(&mut self, index: usize) {
        if index < self.shown.len() {
            self.selected = index;
            self.trash_armed = None;
        }
    }

    pub fn set_filter(&mut self, filter: String) {
        let keep = self.current().map(|e| e.name.clone());
        self.filter = filter;
        self.refilter();
        // The best match first: a name starting with the filter.
        let lower = self.filter.to_lowercase();
        let starts = self.shown.iter().position(|&i| self.entries[i].name.to_lowercase().starts_with(&lower));
        match (starts, keep) {
            (Some(i), _) if !lower.is_empty() => self.selected = i,
            (_, Some(name)) => self.select_name(&name),
            _ => {}
        }
    }

    pub fn toggle_hidden(&mut self) {
        let keep = self.current().map(|e| e.name.clone());
        self.show_hidden = !self.show_hidden;
        self.refilter();
        if let Some(name) = keep {
            self.select_name(&name);
        }
    }

    fn refilter(&mut self) {
        let lower = self.filter.to_lowercase();
        self.shown = (0..self.entries.len())
            .filter(|&i| {
                let e = &self.entries[i];
                // A filter starting with a dot looks for hidden names too.
                (self.show_hidden || !e.hidden() || lower.starts_with('.')) && e.name.to_lowercase().contains(&lower)
            })
            .collect();
        self.selected = self.selected.min(self.shown.len().saturating_sub(1));
    }

    /// Carries out the name being typed: makes the folder or file, or
    /// renames. The new name is chosen afterwards.
    pub fn finish_naming(&mut self) -> Result<String, String> {
        let Some(naming) = self.naming.take() else { return Err("nothing to name".into()) };
        let name = naming.text().trim().to_string();
        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
            return Err(format!("\u{201c}{name}\u{201d} can't be a name here"));
        }
        let target = self.dir.join(&name);
        if target.symlink_metadata().is_ok() && !matches!(&naming, Naming::Rename(from, _) if *from == target) {
            return Err(format!("{name} is already here"));
        }
        let done = match &naming {
            Naming::NewFolder(_) => fs::create_dir(&target).map(|_| format!("made the folder {name}")),
            Naming::NewFile(_) => {
                fs::OpenOptions::new().write(true).create_new(true).open(&target).map(|_| format!("made the file {name}"))
            }
            Naming::Rename(from, _) => fs::rename(from, &target).map(|_| format!("renamed to {name}")),
        };
        let done = done.map_err(|e| e.to_string())?;
        self.filter.clear();
        self.reload();
        self.select_name(&name);
        Ok(done)
    }
}

/// The folder's entries, folders first, then by name (any case).
fn read_dir(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        let Ok(meta) = item.metadata() else { continue };
        let kind = if meta.file_type().is_symlink() {
            Kind::Link(fs::metadata(&path).is_ok_and(|m| m.is_dir()))
        } else if meta.is_dir() {
            Kind::Folder
        } else {
            Kind::File
        };
        // macOS packages (apps, bundles) are folders to the file system but
        // one thing to the user: they open, rather than being gone into.
        let kind = if kind == Kind::Folder && is_package(&name) { Kind::File } else { kind };
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            kind == Kind::File && meta.permissions().mode() & 0o111 != 0
        };
        entries.push(Entry { name, path, kind, size: meta.len(), modified: meta.modified().ok(), executable });
    }
    entries.sort_by(|a, b| b.is_folder().cmp(&a.is_folder()).then_with(|| natural(&a.name, &b.name)));
    Ok(entries)
}

fn is_package(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".app", ".bundle", ".framework", ".photoslibrary", ".pkg", ".xcodeproj", ".xcworkspace", ".playground"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// Names compared as people read them: any case, and numbers by value
/// ("file2" before "file10").
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let number = |s: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = s.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        s.next();
                    }
                    digits
                };
                let (m, n) = (number(&mut a), number(&mut b));
                let (m_trim, n_trim) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let order = m_trim.len().cmp(&n_trim.len()).then_with(|| m_trim.cmp(n_trim));
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// How `path` opens in the terminal: by its name, else by a look inside
/// (text is valid UTF-8 without NULs).
pub fn opener(path: &Path) -> Opener {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "md" | "markdown" | "mdown" | "mkd" | "mdx" => return Opener::Markdown,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => return Opener::Picture,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "key" | "pages" | "numbers" | "mp3" | "mp4" | "mov"
        | "m4a" | "wav" | "aiff" | "zip" | "gz" | "tgz" | "xz" | "bz2" | "7z" | "rar" | "dmg" | "iso" | "app" | "pkg"
        | "heic" | "tiff" | "tif" | "psd" | "ai" | "sketch" | "fig" | "ttf" | "otf" | "woff" | "woff2" | "ico" | "icns"
        | "svg" | "html" | "htm" | "webm" | "mkv" | "avi" | "flac" | "ogg" | "o" | "a" | "dylib" | "so" | "class"
        | "jar" | "wasm" | "sqlite" | "db" => return Opener::App,
        _ => {}
    }
    if name == "readme" && looks_like_text(path) {
        return Opener::Text;
    }
    if looks_like_text(path) { Opener::Text } else { Opener::App }
}

fn looks_like_text(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = fs::File::open(path) else { return false };
    let mut buf = vec![0; SNIFF];
    let Ok(n) = file.read(&mut buf) else { return false };
    let bytes = &buf[..n];
    if bytes.contains(&0) {
        return false;
    }
    // A cut-off character at the end of the sample is still text.
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && n == SNIFF,
    }
}

/// `path` quoted for the shell, relative to `cwd` when it is inside it.
pub fn shell_path(path: &Path, cwd: Option<&Path>) -> String {
    let shown = match cwd.and_then(|c| path.strip_prefix(c).ok()) {
        Some(rel) if !rel.as_os_str().is_empty() => rel.to_path_buf(),
        _ => path.to_path_buf(),
    };
    quote(&shown.to_string_lossy())
}

/// `text` as one word for the shell: bare when it is safe, else in single
/// quotes.
pub fn quote(text: &str) -> String {
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+,:@%".contains(c)) {
        return text.to_string();
    }
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The command that shows `path` in the terminal, for a shell in `cwd`.
pub fn view_command(path: &Path, cwd: Option<&Path>) -> String {
    let arg = shell_path(path, cwd);
    match opener(path) {
        Opener::Markdown => format!("\"$ONEPROMPT_EXE\" md {arg}"),
        Opener::Picture => format!("\"$ONEPROMPT_EXE\" img {arg}"),
        Opener::Text | Opener::App => format!("less -R {arg}"),
    }
}

/// `path` with the home folder as `~`, for showing.
pub fn tilde(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() { "~".into() } else { format!("~/{}", rest.to_string_lossy()) };
    }
    path.to_string_lossy().into_owned()
}

/// A size as people say it: 912 B, 4.2 KB, 18 MB.
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// When something was changed, briefly: "2 min ago", "11 Oct 09:14",
/// "3 Mar 2025".
pub fn when_text(time: SystemTime) -> String {
    let now = SystemTime::now();
    let ago = now.duration_since(time).map(|d| d.as_secs()).unwrap_or(0);
    if ago < 60 {
        return "just now".into();
    }
    if ago < 3600 {
        return format!("{} min ago", ago / 60);
    }
    let local = |t: SystemTime| {
        let secs = t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as libc::time_t).unwrap_or(0);
        // SAFETY: localtime_r fills the zeroed struct it is given.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe { libc::localtime_r(&secs, &mut tm) };
        tm
    };
    let (t, n) = (local(time), local(now));
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let month = MONTHS[t.tm_mon.clamp(0, 11) as usize];
    if t.tm_year == n.tm_year {
        format!("{} {month} {:02}:{:02}", t.tm_mday, t.tm_hour, t.tm_min)
    } else {
        format!("{} {month} {}", t.tm_mday, t.tm_year + 1900)
    }
}

/// Moves `path` to the Trash, as the Finder does (so it can be put back).
#[cfg(target_os = "macos")]
pub fn trash(path: &Path) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let text = NSString::from_str(&path.to_string_lossy());
    let url = NSURL::fileURLWithPath(&text);
    let manager = NSFileManager::defaultManager();
    manager.trashItemAtURL_resultingItemURL_error(&url, None).map_err(|e| e.localizedDescription().to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn trash(_path: &Path) -> Result<(), String> {
    Err("moving to the Trash needs macOS".into())
}

/// Whether the shell in the pane is at its prompt: the process in the
/// foreground is a shell, not a program it started.
pub fn at_prompt(pty: Option<&fs::File>) -> bool {
    use std::os::fd::AsRawFd;
    let Some(pty) = pty else { return false };
    // SAFETY: a plain query on an open descriptor.
    let group = unsafe { libc::tcgetpgrp(pty.as_raw_fd()) };
    if group <= 0 {
        return false;
    }
    let mut name = [0u8; 256];
    // SAFETY: the buffer is as long as the size given.
    let n = unsafe { libc::proc_name(group, name.as_mut_ptr().cast(), name.len() as u32) };
    if n <= 0 {
        return false;
    }
    let name = String::from_utf8_lossy(&name[..n as usize]);
    let name = name.trim_start_matches('-');
    ["zsh", "bash", "fish", "sh", "dash", "ksh", "tcsh", "csh", "nu", "elvish", "xonsh", "login"].contains(&name)
}

/// The width of the browser beside a pane `pane_w` wide, physical pixels:
/// a third of it, between 260 and 400 logical pixels (room for names in
/// the terminal's font), leaving the terminal at least half.
pub fn width(pane_w: f32, scale: f32) -> f32 {
    (pane_w * 0.34).clamp(260.0 * scale, 400.0 * scale).min(pane_w * 0.5)
}

/// A pane's box split into the terminal's and the browser's.
pub fn split(rect: [f32; 4], scale: f32) -> ([f32; 4], [f32; 4]) {
    let [x, y, w, h] = rect;
    let b = width(w, scale);
    ([x, y, w - b, h], [x + w - b, y, b, h])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_as_people_read_them() {
        let mut names = vec!["file10", "File2", "file1", "b", "A"];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(names, ["A", "b", "file1", "File2", "file10"]);
    }

    #[test]
    fn paths_are_quoted_for_the_shell() {
        assert_eq!(quote("src/main.rs"), "src/main.rs");
        assert_eq!(quote("My File.txt"), "'My File.txt'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        let cwd = Path::new("/a/b");
        assert_eq!(shell_path(Path::new("/a/b/c d"), Some(cwd)), "'c d'");
        assert_eq!(shell_path(Path::new("/x/y"), Some(cwd)), "/x/y");
    }

    #[test]
    fn sizes_read_briefly() {
        assert_eq!(size_text(912), "912 B");
        assert_eq!(size_text(4_200), "4.2 KB");
        assert_eq!(size_text(18_000_000), "18 MB");
    }

    #[test]
    fn folders_list_first_and_filter() {
        let dir = std::env::temp_dir().join(format!("oneprompt-browser-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("zeta")).unwrap();
        fs::write(dir.join("alpha.md"), "# hi").unwrap();
        fs::write(dir.join(".hidden"), "x").unwrap();
        fs::write(dir.join("blob.bin"), [0u8, 1, 2]).unwrap();
        let mut b = Browser::new(dir.clone());
        let names: Vec<_> = b.shown().map(|e| e.name.clone()).collect();
        assert_eq!(names, ["zeta", "alpha.md", "blob.bin"]);
        b.toggle_hidden();
        assert_eq!(b.len(), 4);
        b.set_filter("AL".into());
        assert_eq!(b.current().map(|e| e.name.as_str()), Some("alpha.md"));
        assert_eq!(opener(&dir.join("alpha.md")), Opener::Markdown);
        assert_eq!(opener(&dir.join("blob.bin")), Opener::App);
        assert_eq!(opener(&dir.join(".hidden")), Opener::Text);
        b.set_filter(String::new());
        b.naming = Some(Naming::NewFolder("made".into()));
        assert!(b.finish_naming().is_ok());
        assert_eq!(b.current().map(|e| e.name.as_str()), Some("made"));
        b.naming = Some(Naming::Rename(dir.join("made"), "renamed".into()));
        assert!(b.finish_naming().is_ok());
        assert!(dir.join("renamed").is_dir());
        b.enter();
        assert_eq!(b.dir, dir.join("renamed"));
        b.up();
        assert_eq!(b.current().map(|e| e.name.as_str()), Some("renamed"));
        // Esc's way back: up, then into the folder, then where it started.
        assert!(b.back());
        assert_eq!(b.dir, dir.join("renamed"));
        assert!(b.back());
        assert_eq!(b.dir, dir);
        assert_eq!(b.current().map(|e| e.name.as_str()), Some("renamed"));
        assert!(!b.back());
        let _ = fs::remove_dir_all(&dir);
    }
}
