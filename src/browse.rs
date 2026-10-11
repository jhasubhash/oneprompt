//! The app's side of the file browser (browser.rs): opening and closing it
//! beside the focused pane, its keys, clicks, drags and menu items, and
//! keeping it on the shell's folder.

use std::path::{Path, PathBuf};
use std::time::Instant;

use winit::event::{KeyEvent, MouseButton};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

use crate::browser::{self, Browser, Naming, Opener};
use crate::menu::{self, Action};
use crate::renderer::BrowserHit;
use crate::terminal::PaneId;
use crate::{links, App, MULTI_CLICK};

/// A press on an entry that moves further than this (logical pixels) is a
/// drag.
const DRAG: f64 = 6.0;

impl App {
    /// The focused pane's browser.
    pub(crate) fn browser(&self) -> Option<&Browser> {
        self.tabs.get(self.active)?.pane().browser.as_ref()
    }

    pub(crate) fn browser_mut(&mut self) -> Option<&mut Browser> {
        let tab = self.tabs.get_mut(self.active)?;
        let focus = tab.focus;
        tab.panes.iter_mut().find(|p| p.id == focus)?.browser.as_mut()
    }

    /// The focused pane's browser has the keyboard.
    pub(crate) fn browser_has_keys(&self) -> bool {
        self.browser().is_some_and(|b| b.keys && b.closing.is_none())
    }

    /// Cmd+B: opens the browser beside the focused pane, on its shell's
    /// folder; gives it the keyboard if it is open without; closes it if
    /// it has it. From the menu (`strict`) it simply opens or closes.
    pub(crate) fn toggle_browser(&mut self, strict: bool) {
        let theme = self.themes[self.current].clone();
        let time = self.renderer.as_ref().map_or(0.0, |r| r.browser_time(&theme));
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let focus = tab.focus;
        let Some(pane) = tab.panes.iter_mut().find(|p| p.id == focus) else { return };
        let now = Instant::now();
        match pane.browser.as_mut() {
            None => {
                let dir = pane.strikes.cwd().or_else(|| std::env::var_os("HOME").map(PathBuf::from)).unwrap_or_else(|| "/".into());
                pane.browser = Some(Browser::new(dir));
            }
            // Asked back while going: it comes back from where it got to.
            Some(b) if b.closing.is_some() => {
                let closing = b.closing.take().unwrap_or(now);
                let had = closing.saturating_duration_since(b.opened).as_secs_f32().min(time);
                let left = (had - closing.elapsed().as_secs_f32()).max(0.0);
                b.opened = now.checked_sub(std::time::Duration::from_secs_f32(left)).unwrap_or(now);
                b.keys = true;
            }
            Some(b) if !b.keys && !strict => b.keys = true,
            // It goes the way it came, backwards; the terminal widens once
            // it has gone (`finish_browsers`).
            Some(b) if time > 0.0 => {
                b.closing = Some(now);
                b.keys = false;
                b.naming = None;
            }
            Some(_) => pane.browser = None,
        }
        self.relayout();
        self.redraw();
    }

    /// Takes away the browsers that have finished going; true if any did.
    pub(crate) fn finish_browsers(&mut self) -> bool {
        let theme = self.themes[self.current].clone();
        let time = self.renderer.as_ref().map_or(0.0, |r| r.browser_time(&theme));
        let mut gone = false;
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                if pane.browser.as_ref().and_then(|b| b.closing).is_some_and(|c| c.elapsed().as_secs_f32() >= time) {
                    pane.browser = None;
                    gone = true;
                }
            }
        }
        if gone {
            self.relayout();
            self.redraw();
        }
        gone
    }

    /// Looks at every open browser in the window again: its folder may
    /// have changed, or its shell moved to another (the browser follows).
    /// True if any changed.
    pub(crate) fn tick_browsers(&mut self) -> bool {
        let mut changed = false;
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        for pane in &mut tab.panes {
            let Some(b) = pane.browser.as_mut().filter(|b| b.closing.is_none()) else { continue };
            if b.checked.elapsed().as_secs_f32() < browser::RECHECK_SECS {
                continue;
            }
            b.checked = Instant::now();
            let cwd = pane.strikes.cwd();
            match cwd {
                Some(dir) if Some(&dir) != b.shell_dir.as_ref() => {
                    b.shell_dir = Some(dir.clone());
                    // Coming up out of a folder, the one left is chosen.
                    let from = b.dir.strip_prefix(&dir).ok().and_then(|rest| rest.components().next());
                    let from = from.map(|c| c.as_os_str().to_string_lossy().into_owned());
                    b.follow(dir, from.as_deref());
                    changed = true;
                }
                _ => changed |= b.refresh(),
            }
        }
        changed
    }

    /// When the browsers should be looked at again, if any are open.
    pub(crate) fn browsers_due(&self) -> Option<Instant> {
        let tab = self.tabs.get(self.active)?;
        tab.panes
            .iter()
            .filter_map(|p| p.browser.as_ref())
            .map(|b| b.checked + std::time::Duration::from_secs_f32(browser::RECHECK_SECS))
            .min()
    }

    /// A key while the browser has the keyboard; false to let it through
    /// (Cmd shortcuts the browser has no use for).
    pub(crate) fn browser_key(&mut self, event: &KeyEvent) -> bool {
        if !self.browser_has_keys() || self.search.is_some() {
            return false;
        }
        let (cmd, shift, alt, ctrl) = (self.mods.super_key(), self.mods.shift_key(), self.mods.alt_key(), self.mods.control_key());
        let code = match event.physical_key {
            PhysicalKey::Code(code) => Some(code),
            _ => None,
        };
        let named = match &event.logical_key {
            Key::Named(n) => Some(*n),
            _ => None,
        };
        let text = event.text.as_ref().map(|t| t.to_string()).filter(|t| t.chars().all(|c| !c.is_control()));

        // A name being typed takes every key but the window's shortcuts.
        if self.browser().is_some_and(|b| b.naming.is_some()) {
            if cmd && code != Some(KeyCode::KeyV) {
                return false;
            }
            let b = self.browser_mut().expect("browser");
            match named {
                Some(NamedKey::Enter) => {
                    let done = b.finish_naming();
                    match done {
                        Ok(message) => self.show_notice(message, false),
                        Err(err) => self.show_notice(err, true),
                    }
                }
                Some(NamedKey::Escape) => b.naming = None,
                Some(NamedKey::Backspace) => {
                    let t = b.naming.as_mut().expect("naming").text_mut();
                    if alt {
                        // A word at a time.
                        let keep = t.trim_end().rfind([' ', '.', '-', '_']).map_or(0, |i| i + 1);
                        t.truncate(keep);
                    } else {
                        t.pop();
                    }
                }
                _ if cmd => {
                    if let Ok(pasted) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        let line = pasted.lines().next().unwrap_or_default().replace('/', "");
                        b.naming.as_mut().expect("naming").text_mut().push_str(&line);
                    }
                }
                _ => {
                    if let Some(t) = text.filter(|_| !ctrl) {
                        b.naming.as_mut().expect("naming").text_mut().push_str(&t);
                    }
                }
            }
            self.redraw();
            return true;
        }

        if cmd {
            let action = match code {
                Some(KeyCode::KeyC) if alt => Action::BrowserCopyRelative,
                Some(KeyCode::KeyC) if shift => Action::BrowserCopyName,
                Some(KeyCode::KeyC) => Action::BrowserCopyPath,
                Some(KeyCode::KeyO) => Action::BrowserOpenApp,
                Some(KeyCode::KeyE) => Action::BrowserEdit,
                Some(KeyCode::KeyR) => Action::BrowserReveal,
                Some(KeyCode::KeyN) if shift => Action::BrowserNewFolder,
                Some(KeyCode::KeyN) if alt => Action::BrowserNewFile,
                // Cmd+Shift+. as in the Finder's open panels; Cmd+. too.
                Some(KeyCode::Period) => Action::BrowserHidden,
                Some(KeyCode::Backspace) => {
                    self.trash_chosen(true);
                    return true;
                }
                Some(KeyCode::Enter) => Action::BrowserCd,
                Some(KeyCode::ArrowDown) => Action::BrowserOpen,
                Some(KeyCode::ArrowUp) => {
                    self.browser_mut().expect("browser").up();
                    self.redraw();
                    return true;
                }
                _ => return false,
            };
            self.browser_action(action);
            return true;
        }

        let page = 10;
        let b = self.browser_mut().expect("browser");
        match named {
            Some(NamedKey::ArrowUp) if alt => b.select(0),
            Some(NamedKey::ArrowDown) if alt => b.select(b.len().saturating_sub(1)),
            Some(NamedKey::ArrowUp) => b.move_by(-1),
            Some(NamedKey::ArrowDown) => b.move_by(1),
            Some(NamedKey::PageUp) => b.move_by(-page),
            Some(NamedKey::PageDown) => b.move_by(page),
            Some(NamedKey::Home) => b.select(0),
            Some(NamedKey::End) => b.select(b.len().saturating_sub(1)),
            Some(NamedKey::ArrowRight) => {
                b.enter();
            }
            Some(NamedKey::ArrowLeft) => b.up(),
            Some(NamedKey::Enter) => {
                self.browser_action(Action::BrowserOpen);
                return true;
            }
            Some(NamedKey::Tab) => {
                self.browser_action(Action::BrowserInsert);
                return true;
            }
            Some(NamedKey::Space) if b.filter.is_empty() => {
                // A look at it, as the Finder's space bar gives.
                if let Some(path) = b.current().map(|e| e.path.clone()) {
                    quick_look(&path);
                }
            }
            Some(NamedKey::Backspace) if !b.filter.is_empty() => {
                let mut filter = b.filter.clone();
                if alt {
                    filter.clear();
                } else {
                    filter.pop();
                }
                b.set_filter(filter);
            }
            Some(NamedKey::Backspace) => b.up(),
            Some(NamedKey::Escape) if !b.filter.is_empty() => b.set_filter(String::new()),
            // Back the way it came; where it started, it closes.
            Some(NamedKey::Escape) => {
                if !b.back() {
                    self.toggle_browser(true);
                    return true;
                }
            }
            _ if ctrl => match code {
                // Emacs keys move too, as in the shell.
                Some(KeyCode::KeyP) => b.move_by(-1),
                Some(KeyCode::KeyN) => b.move_by(1),
                Some(KeyCode::KeyG) | Some(KeyCode::KeyC) => b.keys = false,
                _ => {}
            },
            _ => {
                if let Some(t) = text {
                    let filter = b.filter.clone() + &t;
                    b.set_filter(filter);
                }
            }
        }
        self.redraw();
        true
    }

    /// Text an input method composed, while the browser has the keyboard.
    pub(crate) fn browser_text(&mut self, text: &str) -> bool {
        if !self.browser_has_keys() {
            return false;
        }
        let b = self.browser_mut().expect("browser");
        match b.naming.as_mut() {
            Some(naming) => naming.text_mut().push_str(text),
            None => {
                let filter = b.filter.clone() + text;
                b.set_filter(filter);
            }
        }
        self.redraw();
        true
    }

    /// A press or release of a mouse button over pane `id`'s browser.
    pub(crate) fn browser_click(&mut self, id: PaneId, pressed: bool, button: MouseButton) {
        if !pressed {
            return;
        }
        if self.tabs.get(self.active).is_some_and(|t| t.focus != id) {
            self.focus_pane(id);
        }
        let hit = self.renderer.as_ref().and_then(|r| r.browser_hit(self.mouse.x as f32, self.mouse.y as f32)).map(|(_, h)| h);
        let now = Instant::now();
        let last = self.mouse.last_browser_click;
        let (x, y) = (self.mouse.x, self.mouse.y);
        let Some(b) = self.browser_mut() else { return };
        b.keys = true;
        b.naming = None;
        match hit {
            Some(BrowserHit::Row(i)) => {
                b.select(i);
                let path = b.current().map(|e| e.path.clone());
                if button == MouseButton::Left {
                    let double = last.is_some_and(|(at, pane, row)| now - at < MULTI_CLICK && pane == id && row == i);
                    self.mouse.last_browser_click = if double { None } else { Some((now, id, i)) };
                    if double {
                        self.browser_action(Action::BrowserOpen);
                        return;
                    }
                    self.mouse.browser_drag = path.map(|p| (p, x, y));
                }
            }
            Some(BrowserHit::Crumb(dir)) => {
                if button == MouseButton::Left {
                    // The folder on the way down to this one is chosen there.
                    let from = b.dir.strip_prefix(&dir).ok().and_then(|rest| rest.components().next());
                    let from = from.map(|c| c.as_os_str().to_string_lossy().into_owned());
                    b.navigate(dir, from.as_deref());
                }
            }
            None => {}
        }
        if button == MouseButton::Right {
            self.show_browser_menu();
        }
        self.redraw();
    }

    /// The left button let go while an entry was held: dropped on a
    /// terminal, its path goes in at the prompt there. True if it was a drag.
    pub(crate) fn browser_drop(&mut self) -> bool {
        let Some((path, x0, y0)) = self.mouse.browser_drag.take() else { return false };
        let scale = self.renderer.as_ref().map_or(1.0, |r| r.window().scale_factor());
        let moved = ((self.mouse.x - x0).powi(2) + (self.mouse.y - y0).powi(2)).sqrt();
        if let Some(r) = self.renderer.as_ref() {
            r.window().set_cursor(winit::window::CursorIcon::Default);
        }
        if moved < DRAG * scale {
            return false;
        }
        let Some(target) = self.pane_under_pointer() else { return true };
        let Some(tab) = self.tabs.get(self.active) else { return true };
        let Some(pane) = tab.panes.iter().find(|p| p.id == target) else { return true };
        let cwd = pane.strikes.cwd();
        pane.terminal.paste(&format!("{} ", browser::shell_path(&path, cwd.as_deref())));
        self.focus_pane(target);
        if let Some(b) = self.tabs[self.active].panes.iter_mut().find(|p| p.id == target).and_then(|p| p.browser.as_mut()) {
            b.keys = false;
        }
        self.redraw();
        true
    }

    /// While an entry is held: the pointer shows it can be dropped.
    pub(crate) fn browser_dragging(&mut self) -> bool {
        let Some((_, x0, y0)) = self.mouse.browser_drag else { return false };
        let Some(r) = self.renderer.as_ref() else { return false };
        let scale = r.window().scale_factor();
        if ((self.mouse.x - x0).powi(2) + (self.mouse.y - y0).powi(2)).sqrt() >= DRAG * scale {
            let over_terminal = r.browser_at(self.mouse.x as f32, self.mouse.y as f32).is_none() && self.pane_under_pointer().is_some();
            r.window().set_cursor(if over_terminal { winit::window::CursorIcon::Copy } else { winit::window::CursorIcon::Grabbing });
        }
        true
    }

    fn show_browser_menu(&mut self) {
        let Some(b) = self.browser() else { return };
        let state = menu::EntryState {
            has_entry: b.current().is_some(),
            folder: b.current().is_some_and(|e| e.is_folder()),
            show_hidden: b.show_hidden,
        };
        if let (Some(m), Some(r)) = (&self.menu, &self.renderer) {
            m.show_entry(r.window(), state);
        }
    }

    /// Carries out one of the browser's menu items (or its key) on the
    /// chosen entry of the focused pane's browser.
    pub(crate) fn browser_action(&mut self, action: Action) {
        let Some(b) = self.browser() else { return };
        let chosen = b.current().cloned();
        let dir = b.dir.clone();
        let shell_dir = self.tabs[self.active].pane().strikes.cwd();
        let path = chosen.as_ref().map(|e| e.path.clone());
        let copy = |text: String| arboard::Clipboard::new().and_then(|mut c| c.set_text(text)).is_ok();
        match action {
            Action::BrowserOpen => {
                let Some(entry) = chosen else { return };
                if entry.is_folder() {
                    self.browser_mut().expect("browser").enter();
                } else if browser::opener(&entry.path) == Opener::App {
                    self.open_in_app(&entry.path, false);
                } else {
                    let command = browser::view_command(&entry.path, shell_dir.as_deref());
                    // The program shown takes the keys, to scroll and quit.
                    if self.run_in_shell(&command) {
                        self.browser_mut().expect("browser").keys = false;
                    }
                }
            }
            Action::BrowserOpenApp => {
                if let Some(path) = path {
                    self.open_in_app(&path, false);
                }
            }
            Action::BrowserReveal => {
                if let Some(path) = path {
                    self.open_in_app(&path, true);
                }
            }
            Action::BrowserEdit => {
                let Some(entry) = chosen.filter(|e| !e.is_folder()) else { return };
                let command = format!("${{EDITOR:-vi}} {}", browser::shell_path(&entry.path, shell_dir.as_deref()));
                if self.run_in_shell(&command) {
                    self.browser_mut().expect("browser").keys = false;
                }
            }
            Action::BrowserCd => {
                // The chosen folder, or the one shown when a file is chosen.
                let target = chosen.filter(|e| e.is_folder()).map_or(dir, |e| e.path);
                self.run_in_shell(&format!("cd {}", browser::shell_path(&target, shell_dir.as_deref())));
            }
            Action::BrowserInsert => {
                if let Some(path) = path {
                    let text = format!("{} ", browser::shell_path(&path, shell_dir.as_deref()));
                    self.tabs[self.active].pane().terminal.paste(&text);
                    self.show_notice(format!("put {} at the prompt", text.trim_end()), false);
                }
            }
            Action::BrowserCopyPath => {
                if let Some(path) = path
                    && copy(path.to_string_lossy().into_owned())
                {
                    self.show_notice(format!("copied {}", browser::tilde(&path)), false);
                }
            }
            Action::BrowserCopyName => {
                if let Some(entry) = chosen
                    && copy(entry.name.clone())
                {
                    self.show_notice(format!("copied {}", entry.name), false);
                }
            }
            Action::BrowserCopyRelative => {
                if let Some(path) = path {
                    let base = shell_dir.unwrap_or(dir);
                    let relative = relative_path(&path, &base);
                    if copy(relative.clone()) {
                        self.show_notice(format!("copied {relative}"), false);
                    }
                }
            }
            Action::BrowserNewFolder | Action::BrowserNewFile | Action::BrowserRename => {
                let naming = match action {
                    Action::BrowserNewFolder => Naming::NewFolder(String::new()),
                    Action::BrowserNewFile => Naming::NewFile(String::new()),
                    _ => match chosen {
                        Some(e) => Naming::Rename(e.path, e.name),
                        None => return,
                    },
                };
                let b = self.browser_mut().expect("browser");
                b.keys = true;
                b.naming = Some(naming);
            }
            Action::BrowserTrash => self.trash_chosen(false),
            Action::BrowserHidden => self.browser_mut().expect("browser").toggle_hidden(),
            _ => {}
        }
        self.redraw();
    }

    /// Moves the chosen entry to the Trash: from a key only when asked
    /// twice in a row (`confirm`), from the menu at once.
    fn trash_chosen(&mut self, confirm: bool) {
        let Some(b) = self.browser_mut() else { return };
        let Some(entry) = b.current().cloned() else { return };
        if confirm && b.trash_armed.as_ref() != Some(&entry.path) {
            b.trash_armed = Some(entry.path.clone());
            self.show_notice(format!("\u{2318}\u{232b} again to move {} to the Trash", entry.name), false);
            return;
        }
        b.trash_armed = None;
        match browser::trash(&entry.path) {
            Ok(()) => {
                b.reload();
                self.show_notice(format!("moved {} to the Trash", entry.name), false);
            }
            Err(err) => self.show_notice(format!("could not move {} to the Trash: {err}", entry.name), true),
        }
        self.redraw();
    }

    /// Types `command` at the focused pane's prompt and runs it, replacing
    /// whatever was typed there; not while a program runs in it (the keys
    /// would go to the program). True if it was sent.
    fn run_in_shell(&mut self, command: &str) -> bool {
        let pane = self.tabs[self.active].pane();
        if !browser::at_prompt(pane.terminal.pty.as_ref()) {
            self.show_notice("the shell is busy with a program: quit it first".into(), true);
            return false;
        }
        // Ctrl+E then Ctrl+U: the line typed so far goes.
        pane.terminal.input(format!("\x05\x15{command}\r").into_bytes());
        true
    }

    /// Opens `path` in its app, or shows it in the Finder (`reveal`).
    fn open_in_app(&mut self, path: &Path, reveal: bool) {
        let result = if reveal {
            std::process::Command::new("open").arg("-R").arg(path).spawn().map(|_| ())
        } else {
            links::open(&links::Target::Path(path.to_path_buf()))
        };
        if let Err(err) = result {
            self.show_notice(format!("could not open {}: {err}", browser::tilde(path)), true);
        }
    }
}

/// Shows `path` in Quick Look, the Finder's preview.
fn quick_look(path: &Path) {
    let _ = std::process::Command::new("qlmanage")
        .arg("-p")
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// `path` relative to `base` (with `..` steps if it is not inside it).
fn relative_path(path: &Path, base: &Path) -> String {
    let (p, b): (Vec<_>, Vec<_>) = (path.components().collect(), base.components().collect());
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c);
    }
    if out.as_os_str().is_empty() { ".".into() } else { out.to_string_lossy().into_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_climb_out_when_needed() {
        assert_eq!(relative_path(Path::new("/a/b/c.txt"), Path::new("/a")), "b/c.txt");
        assert_eq!(relative_path(Path::new("/a/x"), Path::new("/a/b")), "../x");
        assert_eq!(relative_path(Path::new("/a"), Path::new("/a")), ".");
    }
}
