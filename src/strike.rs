//! Struck-out names: when a file or folder that is named on screen is
//! deleted (rm, a script, the Finder, ...), every mention of it is cut
//! through with a sword stroke.
//!
//! The terminal never sees what a command does, so it watches the screen
//! instead: each word that could be a path is looked up relative to the
//! shell's working directory. A path that existed and then disappears is
//! gone; its words are struck from then on, and stop being struck if it
//! comes back. Lookups are cached for a moment, so a frame costs nothing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// How long a lookup stays fresh, in seconds.
const RECHECK: f32 = 0.3;
/// Paths remembered at most; beyond that, ones that never existed go.
const MAX_PATHS: usize = 4096;
/// Most words looked up per frame, so a screen full of text stays cheap.
const MAX_WORDS: usize = 600;

/// One mention of a deleted path on screen: `len` cells from (line, col),
/// to be struck from `born` on (seconds, the renderer's clock).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Struck {
    pub line: usize,
    pub col: usize,
    pub len: usize,
    pub born: f32,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    exists: bool,
    checked: f32,
    /// Seen existing at some point: only those can be deleted.
    existed: bool,
    /// When it was found missing after having existed.
    gone: Option<f32>,
}

/// Per tab: what the words on its screen point to.
pub struct Strikes {
    pty: Option<std::fs::File>,
    cwd: Option<PathBuf>,
    cwd_checked: f32,
    paths: HashMap<PathBuf, Entry>,
}

impl Strikes {
    /// `pty`: the terminal whose foreground process's working directory
    /// words are relative to.
    pub fn new(pty: Option<std::fs::File>) -> Self {
        Self { pty, cwd: None, cwd_checked: f32::NEG_INFINITY, paths: HashMap::new() }
    }

    /// The working directory of the terminal's foreground process, now.
    pub fn cwd(&self) -> Option<PathBuf> {
        self.pty.as_ref().and_then(foreground).and_then(working_directory)
    }

    /// Finds the mentions of deleted paths in the visible grid (`chars`,
    /// row-major, `columns` wide), in reading order.
    pub fn find(&mut self, chars: &[char], columns: usize, now: f32) -> Vec<Struck> {
        if now - self.cwd_checked >= RECHECK {
            self.cwd_checked = now;
            self.cwd = self.pty.as_ref().and_then(foreground).and_then(working_directory);
        }
        let Some(cwd) = self.cwd.clone() else { return Vec::new() };
        let home = std::env::var_os("HOME").map(PathBuf::from);
        if self.paths.len() > MAX_PATHS {
            self.paths.retain(|_, e| e.existed);
        }

        let mut struck = Vec::new();
        for (word, (line, col, len)) in words(chars, columns).into_iter().take(MAX_WORDS) {
            let Some(path) = resolve(&word, &cwd, home.as_deref()) else { continue };
            let entry = self.paths.entry(path.clone()).or_insert(Entry {
                exists: false,
                checked: f32::NEG_INFINITY,
                existed: false,
                gone: None,
            });
            if now - entry.checked >= RECHECK {
                entry.checked = now;
                entry.exists = path.symlink_metadata().is_ok();
                if entry.exists {
                    entry.existed = true;
                    entry.gone = None;
                } else if entry.existed && entry.gone.is_none() {
                    entry.gone = Some(now);
                }
            }
            if let Some(born) = entry.gone {
                struck.push(Struck { line, col, len, born });
            }
        }
        struck
    }
}

/// Words that could be paths, with where they are: (line, col, cells).
/// Words run between spaces, quotes and brackets; trailing punctuation and
/// the marks `ls -F` adds (/ * @) are not part of the name.
fn words(chars: &[char], columns: usize) -> Vec<(String, (usize, usize, usize))> {
    let breaks = |c: char| c.is_whitespace() || "\"'`()[]{}<>|;,=".contains(c);
    let mut out = Vec::new();
    for (line, row) in chars.chunks(columns.max(1)).enumerate() {
        let mut col = 0;
        while col < row.len() {
            if breaks(row[col]) {
                col += 1;
                continue;
            }
            let start = col;
            while col < row.len() && !breaks(row[col]) {
                col += 1;
            }
            let mut end = col;
            while end > start && "/*@:.".contains(row[end - 1]) {
                end -= 1;
            }
            let word: String = row[start..end].iter().collect();
            if !word.is_empty() && !word.starts_with('-') && word != "~" {
                out.push((word, (line, start, end - start)));
            }
        }
    }
    out
}

/// The path a word names: absolute, from home (~/...) or from `cwd`.
fn resolve(word: &str, cwd: &Path, home: Option<&Path>) -> Option<PathBuf> {
    if word == "." || word == ".." {
        return None;
    }
    let path = if let Some(rest) = word.strip_prefix("~/") {
        home?.join(rest)
    } else {
        cwd.join(word)
    };
    // The working directory itself (or "/") is never struck out.
    (path != cwd && path.parent().is_some()).then_some(path)
}

/// The process in the terminal's foreground: the shell at its prompt, or the
/// command it is running.
fn foreground(pty: &std::fs::File) -> Option<u32> {
    use std::os::fd::AsRawFd;
    // SAFETY: a plain query on an open descriptor.
    let group = unsafe { libc::tcgetpgrp(pty.as_raw_fd()) };
    (group > 0).then_some(group as u32)
}

/// A process's current directory, from the kernel.
#[cfg(target_os = "macos")]
fn working_directory(pid: u32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    // SAFETY: the buffer is a zeroed proc_vnodepathinfo of the size given.
    let read = unsafe {
        libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDVNODEPATHINFO, 0, (&raw mut info).cast(), size)
    };
    if read != size {
        return None;
    }
    // SAFETY: vip_path is MAXPATHLEN bytes, NUL-terminated by the kernel.
    let bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(info.pvi_cdir.vip_path.as_ptr().cast(), std::mem::size_of_val(&info.pvi_cdir.vip_path))
    };
    let len = bytes.iter().position(|&b| b == 0)?;
    (len > 0).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(&bytes[..len])))
}

#[cfg(not(target_os = "macos"))]
fn working_directory(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(lines: &[&str], columns: usize) -> Vec<char> {
        lines
            .iter()
            .flat_map(|l| {
                let mut v: Vec<char> = l.chars().collect();
                v.resize(columns, ' ');
                v
            })
            .collect()
    }

    #[test]
    fn words_skip_flags_and_ls_marks() {
        let w = words(&grid(&["$ rm -rf 'old dir' src/ run*", "notes.txt: done."], 30), 30);
        let names: Vec<&str> = w.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(names, ["$", "rm", "old", "dir", "src", "run", "notes.txt", "done"]);
        assert_eq!(w[4].1, (0, 19, 3));
        assert_eq!(w[6].1, (1, 0, 9));
    }

    #[test]
    fn deleted_files_are_struck_and_come_back() {
        let dir = std::env::temp_dir().join(format!("oneprompt-strike-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "").unwrap();
        let mut strikes = Strikes::new(None);
        strikes.cwd = Some(dir.clone());
        strikes.cwd_checked = f32::INFINITY;
        let screen = grid(&["a.txt b.txt", "$ rm a.txt"], 12);

        // Exists: nothing struck. b.txt never existed: never struck.
        assert!(strikes.find(&screen, 12, 0.0).is_empty());
        std::fs::remove_file(dir.join("a.txt")).unwrap();
        // Still cached until the lookup goes stale.
        assert!(strikes.find(&screen, 12, 0.1).is_empty());
        let struck = strikes.find(&screen, 12, 1.0);
        assert_eq!(struck, [
            Struck { line: 0, col: 0, len: 5, born: 1.0 },
            Struck { line: 1, col: 5, len: 5, born: 1.0 },
        ]);
        // Made again: no longer struck.
        std::fs::write(dir.join("a.txt"), "").unwrap();
        assert!(strikes.find(&screen, 12, 2.0).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
