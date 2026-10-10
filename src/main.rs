//! oneprompt: a terminal where the background and the type are part of the design.
//!
//! The shell runs in a PTY managed by alacritty_terminal (see terminal.rs);
//! this file wires window events to it and asks the renderer to draw the grid
//! with the current theme.
//!
//! Usage:
//!   oneprompt                 run your login shell
//!   oneprompt -e "command"    run a command through /bin/sh instead
//!   oneprompt --position X,Y  open the window there (logical pixels; Cmd+N)
//!   oneprompt --default-config   print the settings file Cmd+, would create
//!   ONEPROMPT_SNAPSHOT=dir    render every theme to dir/<theme>.png and exit
//!   ONEPROMPT_SNAPSHOT_SWITCH=n   with it, also n frames of each switch

mod animation;
mod boxdraw;
mod chrome;
mod color;
mod config;
mod input;
mod links;
mod marks;
mod menu;
mod panes;
mod picker;
mod renderer;
mod search;
mod settings;
mod strike;
mod terminal;
mod theme;
mod themes;
mod trace;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersKeyState, ModifiersState};
use winit::window::{Window, WindowId};

use chrome::Control;
use config::{Config, OptionAsMeta};
use menu::Action;
use renderer::{Notice, Renderer};
use terminal::{PaneId, Terminal, UserEvent};
use theme::Variant;

/// How long snapshot mode lets the command run before capturing.
const SNAPSHOT_DELAY: Duration = Duration::from_millis(1500);

struct App {
    proxy: EventLoopProxy<UserEvent>,
    command: Option<String>,
    snapshot_dir: Option<PathBuf>,
    started: Instant,

    /// Where to open the window (Cmd+N cascades new windows).
    position: Option<(f64, f64)>,

    renderer: Option<Renderer>,
    /// Each tab runs its own shell; the window shows the active one.
    tabs: Vec<Tab>,
    active: usize,
    /// The id the next pane gets (each pane runs its own shell).
    next_pane: PaneId,
    /// The tab name being edited in the strip.
    renaming: Option<Rename>,
    menu: Option<menu::ContextMenu>,
    /// Every theme's variants, with the settings applied.
    themes: Vec<Variant>,
    current: usize,
    /// The theme picker is open, with this theme (index in `theme::all()`)
    /// chosen.
    picker: Option<usize>,
    /// The settings panel, while it is open.
    settings: Option<SettingsState>,
    /// The settings file as the panel last wrote it, already applied: the
    /// watcher noticing that write has nothing to do.
    written: Option<String>,
    /// Why the settings file could not be applied, while it cannot.
    config_error: Option<String>,
    /// Where the input method's candidate window was last told to go.
    ime_area: [f32; 4],
    /// The link under the pointer while Cmd is held, and the cell it was
    /// looked up for (screen column and line).
    link: Option<(links::Link, (usize, usize))>,
    /// Find, while its bar is open; the text being composed for it in an
    /// input method; and whether the terminal has changed since the
    /// matches were found, and when they were.
    search: Option<search::Search>,
    search_preedit: String,
    search_stale: bool,
    searched_at: Instant,
    mods: ModifiersState,
    /// Which Option keys are held right now: (left, right).
    option_keys: (bool, bool),
    config_path: PathBuf,
    config: Config,
    /// When the notice should disappear.
    notice_until: Option<Instant>,
    /// macOS is in dark mode.
    system_dark: bool,
    /// Cmd+Shift+L override for this session: Some(true) = dark.
    flip: Option<bool>,
    /// Leftover trackpad scroll, in pixels, below one line.
    scroll_px: f64,
    mouse: Mouse,
    /// Set by Cmd+W or the close control; the event handler then exits.
    close_requested: bool,
    /// A settings error found before the window existed, shown once it does.
    startup_error: Option<String>,
}

/// The settings panel's rows, the one the keyboard is on, and the slider
/// being dragged (its row, and its track's left edge and width).
struct SettingsState {
    items: Vec<settings::Item>,
    focus: usize,
    drag: Option<(usize, f32, f32)>,
}

/// A tab name being typed: which tab, the text so far, and whether all of
/// it is selected (typing then replaces it, as in any text field).
struct Rename {
    index: usize,
    text: String,
    all: bool,
}

/// A tab: one or more panes (splits), one of them with the keyboard.
struct Tab {
    panes: Vec<Pane>,
    layout: panes::Layout,
    focus: PaneId,
    /// Given by the user (Rename Tab); wins over the title.
    name: Option<String>,
}

/// A shell in a tab.
struct Pane {
    id: PaneId,
    terminal: Terminal,
    /// Set by the program running in it (OSC 0/2).
    title: Option<String>,
    /// Names on its screen whose files were deleted.
    strikes: strike::Strikes,
}

impl Pane {
    fn new(id: PaneId, terminal: Terminal) -> Self {
        let strikes = strike::Strikes::new(terminal.pty.as_ref().and_then(|f| f.try_clone().ok()));
        Self { id, terminal, title: None, strikes }
    }
}

impl Tab {
    fn new(pane: Pane) -> Self {
        Self { layout: panes::Layout::Pane(pane.id), focus: pane.id, panes: vec![pane], name: None }
    }

    /// The pane with the keyboard.
    fn pane(&self) -> &Pane {
        self.panes.iter().find(|p| p.id == self.focus).unwrap_or(&self.panes[0])
    }

    /// The tab's title: its focused pane's.
    fn title(&self) -> Option<&str> {
        self.pane().title.as_deref()
    }
}

/// How wide a divider between panes is to grab, in logical pixels.
const DIVIDER_GRAB: f32 = 8.0;

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
    /// Window control pressed; it acts on release over the same control.
    pressed_control: Option<Control>,
    /// Last press in the title strip, for double-click to zoom.
    last_strip_click: Option<Instant>,
    /// Last click on a tab, for double-click to rename.
    last_tab_click: Option<(Instant, usize)>,
    /// The divider between panes being dragged.
    divider: Option<panes::Divider>,
}

/// Clicks closer together than this count as a double/triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);

impl App {
    fn theme(&self) -> &Variant {
        &self.themes[self.current]
    }

    /// The terminal of the active tab's focused pane.
    fn terminal(&self) -> Option<&Terminal> {
        self.tabs.get(self.active).map(|t| &t.pane().terminal)
    }

    /// The active tab's panes and their boxes in the window, physical
    /// pixels, in reading order.
    fn pane_rects(&self) -> Vec<(PaneId, [f32; 4])> {
        let (Some(r), Some(tab)) = (self.renderer.as_ref(), self.tabs.get(self.active)) else { return Vec::new() };
        tab.layout.rects(r.window_rect())
    }

    /// The box of the active tab's focused pane.
    fn focus_rect(&self) -> Option<[f32; 4]> {
        let focus = self.tabs.get(self.active)?.focus;
        self.pane_rects().into_iter().find(|(id, _)| *id == focus).map(|(_, rect)| rect)
    }

    /// Where the tab and pane with this id are.
    fn find_pane(&self, id: PaneId) -> Option<(usize, usize)> {
        self.tabs
            .iter()
            .enumerate()
            .find_map(|(t, tab)| tab.panes.iter().position(|p| p.id == id).map(|p| (t, p)))
    }

    /// Recomputes every pane's grid for the window and theme, and tells the
    /// shells. Every tab is laid out for the window, so switching never
    /// resizes.
    fn relayout(&mut self) {
        let theme = self.themes[self.current].clone();
        let Some(renderer) = self.renderer.as_mut() else { return };
        let window = renderer.window_rect();
        for tab in &mut self.tabs {
            for (id, rect) in tab.layout.rects(window) {
                let size = renderer.grid_size(&theme, id, rect);
                if let Some(pane) = tab.panes.iter_mut().find(|p| p.id == id) {
                    pane.terminal.resize(size);
                }
            }
        }
        renderer.window().request_redraw();
    }

    /// Starts a shell in pane `id` at `rect`, in `cwd` (None: home).
    fn spawn_pane(&mut self, id: PaneId, rect: [f32; 4], cwd: Option<PathBuf>) -> std::io::Result<Pane> {
        let theme = self.themes[self.current].clone();
        let Some(renderer) = self.renderer.as_mut() else { return Err(std::io::Error::other("no window")) };
        let size = renderer.grid_size(&theme, id, rect);
        let integrate = self.config.shell_integration.unwrap_or(true);
        let terminal = Terminal::spawn(self.proxy.clone(), id, size, self.command.clone(), cwd, integrate)?;
        Ok(Pane::new(id, terminal))
    }

    /// Starts a shell in a new tab and shows it.
    fn new_tab(&mut self) -> std::io::Result<()> {
        let Some(window) = self.renderer.as_ref().map(|r| r.window_rect()) else { return Ok(()) };
        let id = self.next_pane;
        self.next_pane += 1;
        let pane = self.spawn_pane(id, window, None)?;
        self.tabs.push(Tab::new(pane));
        self.switch_tab(self.tabs.len() - 1);
        Ok(())
    }

    /// Splits the focused pane in two, side by side (`across`) or one above
    /// the other; the new half runs a new shell in the same folder and gets
    /// the keyboard.
    fn split(&mut self, across: bool) -> std::io::Result<()> {
        let Some(tab) = self.tabs.get(self.active) else { return Ok(()) };
        let (focus, cwd) = (tab.focus, tab.pane().strikes.cwd());
        let mut layout = tab.layout.clone();
        let id = self.next_pane;
        self.next_pane += 1;
        layout.split(focus, id, across);
        let Some(window) = self.renderer.as_ref().map(|r| r.window_rect()) else { return Ok(()) };
        let rect = layout.rects(window).into_iter().find(|(p, _)| *p == id).map_or(window, |(_, r)| r);
        let pane = self.spawn_pane(id, rect, cwd)?;
        let tab = &mut self.tabs[self.active];
        tab.layout = layout;
        tab.panes.push(pane);
        // Writing in starts over: the old panes are not new ink.
        if let Some(r) = self.renderer.as_mut() {
            r.reset_writing();
        }
        self.relayout();
        self.focus_pane(id);
        Ok(())
    }

    /// Gives pane `id` of the active tab the keyboard.
    fn focus_pane(&mut self, id: PaneId) {
        let window_focused = self.renderer.as_ref().is_some_and(|r| r.window().has_focus());
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if !tab.panes.iter().any(|p| p.id == id) {
            return;
        }
        tab.focus = id;
        for pane in &tab.panes {
            pane.terminal.set_focused(window_focused && pane.id == id);
        }
        self.mouse.selecting = false;
        self.mouse.reported_button = None;
        self.mouse.reported_cell = None;
        if let Some(search) = self.search.as_mut() {
            search.refresh(&self.tabs[self.active].pane().terminal.term.lock());
        }
        self.update_tabs();
        self.redraw();
    }

    /// Closes the focused pane, or the tab with its last one; returns false
    /// when that was the window's last tab.
    fn close_pane(&mut self) -> bool {
        let Some(tab) = self.tabs.get(self.active) else { return true };
        if tab.panes.len() < 2 {
            return self.close_tab(self.active);
        }
        let focus = tab.focus;
        self.remove_pane(self.active, focus);
        true
    }

    /// Takes pane `id` out of tab `index`, which has another; the pane
    /// beside it in reading order gets the keyboard if it had it.
    fn remove_pane(&mut self, index: usize, id: PaneId) {
        let tab = &mut self.tabs[index];
        let order = tab.layout.ids();
        let at = order.iter().position(|p| *p == id).unwrap_or(0);
        tab.layout.remove(id);
        tab.panes.retain(|p| p.id != id);
        let next = order.iter().filter(|p| **p != id).nth(at.saturating_sub(1)).copied();
        if tab.focus == id
            && let Some(next) = next
        {
            tab.focus = next;
        }
        if let Some(r) = self.renderer.as_mut() {
            r.forget_pane(id);
        }
        self.relayout();
        if index == self.active {
            let focus = self.tabs[index].focus;
            self.focus_pane(focus);
        }
    }

    /// Moves the keyboard to the pane next to the focused one that way.
    fn focus_toward(&mut self, direction: (i32, i32)) {
        let Some(focus) = self.tabs.get(self.active).map(|t| t.focus) else { return };
        if let Some(id) = panes::neighbour(&self.pane_rects(), focus, direction) {
            self.focus_pane(id);
        }
    }

    /// Moves the keyboard to the next (or previous) pane in reading order.
    fn focus_next(&mut self, by: i32) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let order = tab.layout.ids();
        let at = order.iter().position(|p| *p == tab.focus).unwrap_or(0) as i32;
        let next = order[(at + by).rem_euclid(order.len() as i32) as usize];
        self.focus_pane(next);
    }

    /// Shows tab `index`.
    fn switch_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.finish_rename(true);
        if let Some(old) = self.tabs.get(self.active).filter(|_| self.active != index) {
            for pane in &old.panes {
                pane.terminal.set_focused(false);
            }
        }
        self.active = index;
        self.mouse.selecting = false;
        self.mouse.reported_button = None;
        self.mouse.reported_cell = None;
        let focused = self.renderer.as_ref().is_some_and(|r| r.window().has_focus());
        let tab = &self.tabs[index];
        for pane in &tab.panes {
            pane.terminal.set_focused(focused && pane.id == tab.focus);
        }
        if let Some(r) = self.renderer.as_mut() {
            // Another tab's screen is not new ink: show it as it stands.
            r.reset_writing();
        }
        self.update_tabs();
        // Find looks in the tab on show.
        if let Some(search) = self.search.as_mut()
            && let Some(tab) = self.tabs.get(index)
        {
            search.refresh(&tab.pane().terminal.term.lock());
        }
        // A full-screen app in the new tab's panes may need its text shrunk.
        if let (Some(r), Some(tab)) = (self.renderer.as_mut(), self.tabs.get(index)) {
            for pane in &tab.panes {
                r.set_full_screen(pane.id, pane.terminal.mode().contains(TermMode::ALT_SCREEN));
            }
        }
        self.relayout();
    }

    /// Closes tab `index`; returns false when it was the last one.
    fn close_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return true;
        }
        self.renaming = None;
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            return false;
        }
        let active = if self.active > index || self.active >= self.tabs.len() { self.active - 1 } else { self.active };
        self.active = usize::MAX;
        self.switch_tab(active.min(self.tabs.len() - 1));
        true
    }

    /// Tells the renderer the tab labels and sets the window title. Labels
    /// show with several tabs, or with one that has a name (or is getting one).
    fn update_tabs(&mut self) {
        let mut labels: Vec<String> = self.tabs.iter().enumerate().map(|(i, t)| tab_label(t, i)).collect();
        let editing = self.renaming.as_ref().map(|r| (r.index, r.all));
        if let Some(rename) = &self.renaming
            && let Some(label) = labels.get_mut(rename.index)
        {
            label.clone_from(&rename.text);
        }
        let show = labels.len() > 1 || editing.is_some() || self.tabs.iter().any(|t| t.name.is_some());
        if let Some(r) = self.renderer.as_mut() {
            if let Some(tab) = self.tabs.get(self.active) {
                r.window().set_title(&tab_label(tab, self.active));
            }
            r.set_tabs(if show { labels } else { Vec::new() }, self.active, editing);
            r.window().request_redraw();
        }
    }

    /// Starts editing tab `index`'s name in the strip.
    fn start_rename(&mut self, index: usize) {
        let Some(tab) = self.tabs.get(index) else { return };
        let text = tab.name.clone().unwrap_or_else(|| tab_label(tab, index));
        // Starts with the whole name selected, as renaming in Finder does.
        self.renaming = Some(Rename { index, text, all: true });
        self.update_tabs();
    }

    /// Ends editing: keeps the name (an empty one goes back to the
    /// automatic title) or drops the edit.
    fn finish_rename(&mut self, keep: bool) {
        let Some(Rename { index, text, .. }) = self.renaming.take() else { return };
        if keep && let Some(tab) = self.tabs.get_mut(index) {
            let text = text.trim();
            tab.name = (!text.is_empty()).then(|| text.to_string());
        }
        self.update_tabs();
    }

    /// Keys while a tab name is being edited. Text editing keys (Cmd+A, C,
    /// X, V included) act on the name; returns false for other Cmd
    /// shortcuts, which keep the name and then go on to the usual handling.
    fn rename_key(&mut self, event: &KeyEvent) -> bool {
        use winit::keyboard::NamedKey;
        let Some(rename) = self.renaming.as_mut() else { return false };
        if self.mods.super_key() {
            let key = match &event.logical_key {
                Key::Character(c) => c.to_lowercase(),
                _ => String::new(),
            };
            match key.as_str() {
                "a" => rename.all = true,
                "c" | "x" => {
                    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(rename.text.clone()));
                    if key == "x" && rename.all {
                        rename.text.clear();
                        rename.all = false;
                    }
                }
                "v" => {
                    if let Ok(pasted) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        if rename.all {
                            rename.text.clear();
                            rename.all = false;
                        }
                        // One line: newlines and tabs become spaces.
                        rename.text.extend(pasted.chars().map(|c| if c.is_control() { ' ' } else { c }));
                    }
                }
                _ => {
                    self.finish_rename(true);
                    return false;
                }
            }
            self.update_tabs();
            return true;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Enter | NamedKey::Escape) => {
                let keep = event.logical_key == Key::Named(NamedKey::Enter);
                self.finish_rename(keep);
                return true;
            }
            Key::Named(NamedKey::Backspace) => {
                if rename.all {
                    rename.text.clear();
                } else {
                    rename.text.pop();
                }
                rename.all = false;
            }
            // The caret stays at the end; moving it just drops the selection.
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight | NamedKey::Home | NamedKey::End) => rename.all = false,
            _ => {
                if let Some(typed) = event.text.as_deref().filter(|t| !t.chars().any(char::is_control)) {
                    if rename.all {
                        rename.text.clear();
                        rename.all = false;
                    }
                    rename.text.push_str(typed);
                }
            }
        }
        self.update_tabs();
        true
    }

    /// Shows the right-click menu at the pointer.
    fn show_menu(&mut self) {
        let state = menu::State {
            has_selection: self.terminal().and_then(|t| t.selection_text()).is_some(),
            dark: self.themes[self.current].dark,
            tabs: self.tabs.len(),
            panes: self.tabs.get(self.active).map_or(1, |t| t.panes.len()),
        };
        if let (Some(m), Some(r)) = (&self.menu, &self.renderer) {
            m.show(r.window(), state);
        }
    }

    /// Carries out a menu item or its shortcut.
    fn menu_action(&mut self, action: Action) {
        match action {
            Action::Copy => {
                if let Some(text) = self.terminal().and_then(|t| t.selection_text()) {
                    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
                }
            }
            Action::Paste => {
                let text = arboard::Clipboard::new().and_then(|mut c| c.get_text());
                if let (Ok(text), Some(t)) = (text, self.terminal()) {
                    t.paste(&text);
                }
            }
            Action::SelectAll => {
                if let Some(rename) = self.renaming.as_mut() {
                    rename.all = true;
                    self.update_tabs();
                } else if let Some(t) = self.terminal() {
                    t.select_all();
                }
            }
            Action::Clear => {
                if let Some(t) = self.terminal() {
                    t.clear();
                }
                if let Some(r) = self.renderer.as_mut() {
                    r.reset_writing();
                }
            }
            Action::NewTab => {
                if let Err(err) = self.new_tab() {
                    self.show_notice(format!("could not open a tab: {err}"), true);
                }
            }
            Action::NewWindow => self.new_window(),
            Action::SplitRight | Action::SplitDown => {
                if let Err(err) = self.split(action == Action::SplitRight) {
                    self.show_notice(format!("could not split: {err}"), true);
                }
            }
            Action::RenameTab => self.start_rename(self.active),
            Action::CloseTab => {
                if !self.close_pane() {
                    self.close_requested = true;
                }
            }
            Action::Bigger | Action::Smaller | Action::ActualSize => {
                if let Some(r) = self.renderer.as_mut() {
                    match action {
                        Action::Bigger => r.zoom_by(1.1),
                        Action::Smaller => r.zoom_by(1.0 / 1.1),
                        _ => r.reset_zoom(),
                    }
                }
                self.relayout();
            }
            Action::FlipAppearance => {
                self.flip = Some(!self.themes[self.current].dark);
                self.select_theme();
                // The picker's cards show the new appearance too.
                self.update_picker();
            }
            Action::Find => {
                if self.search.is_none() {
                    self.close_picker();
                    self.close_settings();
                    self.search = Some(search::Search::default());
                }
                self.update_search(false);
            }
            Action::Themes => {
                if self.picker.is_some() {
                    self.close_picker();
                } else {
                    self.close_search();
                    self.close_settings();
                    let id = self.themes[self.current].theme.id();
                    self.picker = Some(theme::all().iter().position(|t| t.id() == id).unwrap_or(0));
                    self.update_picker();
                }
            }
            Action::Settings => {
                if self.settings.is_some() {
                    self.close_settings();
                } else {
                    self.close_picker();
                    self.close_search();
                    self.settings = Some(SettingsState { items: Vec::new(), focus: 0, drag: None });
                    self.update_settings();
                    if let Some(state) = self.settings.as_mut() {
                        state.focus = settings::first_row(&state.items);
                    }
                    self.update_settings();
                }
            }
            Action::SettingsFile => {
                if let Err(err) = config::open_in_editor(&self.config_path) {
                    self.show_notice(format!("could not open the settings file: {err}"), true);
                }
            }
        }
        self.redraw();
    }

    /// Opens another window: a separate oneprompt, a little down and to the
    /// right of this one.
    fn new_window(&mut self) {
        let mut command = match std::env::current_exe() {
            Ok(exe) => std::process::Command::new(exe),
            Err(err) => return self.show_notice(format!("could not open a window: {err}"), true),
        };
        if let Some(r) = &self.renderer {
            let window = r.window();
            if let Ok(at) = window.outer_position() {
                let at = at.to_logical::<f64>(window.scale_factor());
                command.arg("--position").arg(format!("{},{}", at.x + 28.0, at.y + 28.0));
            }
        }
        if let Err(err) = command.spawn() {
            self.show_notice(format!("could not open a window: {err}"), true);
        }
    }

    fn redraw(&self) {
        if let Some(r) = &self.renderer {
            r.window().request_redraw();
        }
    }

    /// App shortcuts (Cmd+...). Returns true if the key was handled.
    /// Shows the picker as it is now: per theme, the variant it would show
    /// for the current appearance.
    fn update_picker(&mut self) {
        let Some(selected) = self.picker else { return };
        let previews: Vec<Variant> = theme::all()
            .iter()
            .map(|t| {
                let i = self.config.theme_index(&self.themes, self.system_dark, self.flip, Some(t.id()));
                self.themes[i].clone()
            })
            .collect();
        if let Some(r) = self.renderer.as_mut() {
            r.set_picker(Some((selected, previews)));
            r.window().request_redraw();
        }
    }

    fn close_picker(&mut self) {
        self.picker = None;
        if let Some(r) = self.renderer.as_mut() {
            r.set_picker(None);
            r.window().request_redraw();
        }
    }

    /// Switches to theme `index` (in `theme::all()`), saving it in the
    /// settings file.
    fn apply_picked(&mut self, index: usize) {
        self.close_picker();
        let Some(theme) = theme::all().get(index).copied() else { return };
        let edit = config::Edit { section: None, key: "theme".into(), value: config::Value::Str(theme.id().into()) };
        self.write_settings(&[edit]);
    }

    /// Shows the settings panel as the settings are now, keeping the
    /// keyboard on the same row (the rows change with the theme).
    fn update_settings(&mut self) {
        let variant = self.themes[self.current].clone();
        let (Some(state), Some(r)) = (self.settings.as_mut(), self.renderer.as_mut()) else { return };
        let families = r.monospace_families();
        let weights = r.font_weights(&variant.font_family);
        let items = settings::items(&self.config, &variant, &settings::Fonts { families: &families, weights: &weights });
        let label = state.items.get(state.focus).and_then(settings::Item::row).map(|row| row.label.clone());
        state.focus = label
            .and_then(|label| items.iter().position(|i| i.row().is_some_and(|row| row.label == label)))
            .unwrap_or_else(|| settings::first_row(&items));
        state.items = items;
        let path = self.config_path.display().to_string();
        let path = match std::env::var("HOME") {
            Ok(home) if !home.is_empty() && path.starts_with(&home) => format!("~{}", &path[home.len()..]),
            _ => path,
        };
        r.set_settings(Some(renderer::SettingsPanel {
            items: state.items.clone(),
            focus: state.focus,
            path,
            error: self.config_error.clone(),
        }));
        r.window().request_redraw();
    }

    fn close_settings(&mut self) {
        if self.settings.take().is_some()
            && let Some(r) = self.renderer.as_mut()
        {
            r.set_settings(None);
            r.window().request_redraw();
        }
    }

    /// Writes `edits` to the settings file and applies the result, unless
    /// the settings would then be wrong (or the file cannot be read).
    fn write_settings(&mut self, edits: &[config::Edit]) {
        let result = config::read_or_template(&self.config_path)
            .map_err(|e| e.to_string())
            .and_then(|text| config::apply_edits(&text, edits))
            .and_then(|text| Config::parse(&text).map(|config| (text, config)));
        let (text, config) = match result {
            Ok(done) => done,
            Err(err) => return self.show_notice(format!("not saved: {err}"), true),
        };
        if let Err(err) = config::write(&self.config_path, &text) {
            return self.show_notice(format!("not saved: {err}"), true);
        }
        self.written = Some(text);
        self.config_error = None;
        self.apply_config(config);
        self.check_font();
        self.update_settings();
    }

    /// Makes `change` to settings row `index`.
    fn settings_change(&mut self, index: usize, change: settings::Change) {
        let Some(state) = self.settings.as_mut() else { return };
        state.focus = index;
        let edits = state.items.get(index).and_then(settings::Item::row).map(|row| row.change(change)).unwrap_or_default();
        if edits.is_empty() {
            self.update_settings();
        } else {
            self.write_settings(&edits);
        }
    }

    /// Keys while the settings panel is open: arrows move between rows and
    /// change them. Command shortcuts still work (Cmd+, closes it, Cmd+Shift+L
    /// shows the other variant's settings); nothing reaches the shell.
    fn settings_key(&mut self, event: &KeyEvent) -> bool {
        use settings::Change;
        use winit::keyboard::NamedKey;
        let Some(state) = self.settings.as_mut() else { return false };
        if self.mods.super_key() {
            if event.logical_key == Key::Character("o".into()) {
                self.menu_action(Action::SettingsFile);
                return true;
            }
            return false;
        }
        let (focus, shift) = (state.focus, self.mods.shift_key());
        let moved = |by| settings::next_row(&state.items, focus, by);
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.close_settings(),
            Key::Named(NamedKey::ArrowUp) => state.focus = moved(-1),
            Key::Named(NamedKey::ArrowDown) => state.focus = moved(1),
            Key::Named(NamedKey::Tab) => state.focus = moved(if shift { -1 } else { 1 }),
            Key::Named(NamedKey::ArrowLeft) => self.settings_change(focus, Change::Step { by: -1, fine: shift }),
            Key::Named(NamedKey::ArrowRight) => self.settings_change(focus, Change::Step { by: 1, fine: shift }),
            Key::Named(NamedKey::Enter | NamedKey::Space) => self.settings_change(focus, Change::Next),
            _ => {}
        }
        self.update_settings();
        true
    }

    /// A click or release on the settings panel; a click beside it closes it.
    fn settings_button(&mut self, pressed: bool) {
        use settings::{Change, Hit, Part};
        let (x, y) = (self.mouse.x as f32, self.mouse.y as f32);
        if !pressed {
            if let Some(state) = self.settings.as_mut() {
                state.drag = None;
            }
            return;
        }
        let Some(r) = self.renderer.as_ref() else { return };
        let (hit, inside) = (r.settings_hit(x, y), r.settings_contains(x, y));
        match hit {
            Some(Hit::OpenFile) => self.menu_action(Action::SettingsFile),
            Some(Hit::Row(index, part)) => match part {
                Part::Label => {
                    if let Some(state) = self.settings.as_mut() {
                        state.focus = index;
                    }
                    self.update_settings();
                }
                Part::Option(i) => self.settings_change(index, Change::Pick(i)),
                Part::Previous => self.settings_change(index, Change::Step { by: -1, fine: false }),
                Part::Next => self.settings_change(index, Change::Step { by: 1, fine: false }),
                Part::Toggle => self.settings_change(index, Change::Next),
                Part::Slider(left, width) => {
                    if let Some(state) = self.settings.as_mut() {
                        state.drag = Some((index, left, width));
                    }
                    self.settings_change(index, Change::Along((x - left) / width.max(1.0)));
                }
            },
            None if !inside => self.close_settings(),
            None => {}
        }
    }

    /// The slider being dragged follows the pointer.
    fn settings_drag(&mut self) {
        let Some((index, left, width)) = self.settings.as_ref().and_then(|s| s.drag) else { return };
        let along = (self.mouse.x as f32 - left) / width.max(1.0);
        self.settings_change(index, settings::Change::Along(along));
    }

    /// Keys while the picker is open. Command shortcuts still work (so
    /// Cmd+Shift+L flips light and dark); nothing reaches the shell.
    fn picker_key(&mut self, event: &KeyEvent) -> bool {
        use winit::keyboard::NamedKey;
        let Some(selected) = self.picker else { return false };
        if self.mods.super_key() {
            return false;
        }
        let n = theme::all().len();
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.close_picker(),
            Key::Named(NamedKey::Enter) => self.apply_picked(selected),
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown) => {
                self.picker = Some((selected + 1) % n);
                self.update_picker();
            }
            Key::Named(NamedKey::Tab) => {
                self.picker = Some(if self.mods.shift_key() { (selected + n - 1) % n } else { (selected + 1) % n });
                self.update_picker();
            }
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => {
                self.picker = Some((selected + n - 1) % n);
                self.update_picker();
            }
            _ => {}
        }
        true
    }

    /// The picker card under the pointer.
    fn picker_card(&self) -> Option<usize> {
        let r = self.renderer.as_ref()?;
        picker::card_at(&r.picker_cards(), self.mouse.x as f32, self.mouse.y as f32, r.window().scale_factor() as f32)
    }

    fn shortcut(&mut self, event: &KeyEvent) -> bool {
        if !self.mods.super_key() {
            return false;
        }
        // Cmd+Option+T: Option changes the character, so go by the key.
        if self.mods.alt_key() && event.physical_key == winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyT) {
            self.menu_action(Action::Themes);
            return true;
        }
        // Cmd+Option+arrows: the pane that way gets the keyboard.
        if self.mods.alt_key()
            && let Key::Named(named) = &event.logical_key
        {
            use winit::keyboard::NamedKey;
            let direction = match named {
                NamedKey::ArrowLeft => (-1, 0),
                NamedKey::ArrowRight => (1, 0),
                NamedKey::ArrowUp => (0, -1),
                NamedKey::ArrowDown => (0, 1),
                _ => return false,
            };
            self.focus_toward(direction);
            return true;
        }
        let Key::Character(key) = &event.logical_key else {
            return false;
        };
        let shift = self.mods.shift_key();
        let action = match key.as_str() {
            // Flip between the light and dark variant for this session.
            "l" | "L" if shift => Some(Action::FlipAppearance),
            "t" => Some(Action::NewTab),
            "n" => Some(Action::NewWindow),
            "d" if !shift => Some(Action::SplitRight),
            "d" | "D" => Some(Action::SplitDown),
            "=" | "+" => Some(Action::Bigger),
            "-" => Some(Action::Smaller),
            "0" => Some(Action::ActualSize),
            "," => Some(Action::Settings),
            "w" => Some(Action::CloseTab),
            "c" => Some(Action::Copy),
            "v" => Some(Action::Paste),
            "a" => Some(Action::SelectAll),
            "k" => Some(Action::Clear),
            "f" => Some(Action::Find),
            _ => None,
        };
        if let Some(action) = action {
            self.menu_action(action);
            return true;
        }
        match key.as_str() {
            // Next / previous tab (Shift+] and Shift+[ arrive as braces).
            "}" | "]" if shift => {
                let n = self.tabs.len();
                self.switch_tab((self.active + 1) % n.max(1));
            }
            "{" | "[" if shift => {
                let n = self.tabs.len();
                self.switch_tab((self.active + n.max(1) - 1) % n.max(1));
            }
            // Cmd+] / Cmd+[: the next / previous pane.
            "]" => self.focus_next(1),
            "[" => self.focus_next(-1),
            // Cmd+1..8 pick a tab, Cmd+9 the last one.
            digit @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9") => {
                let n: usize = digit.parse().unwrap_or(1);
                let index = if n == 9 { self.tabs.len().saturating_sub(1) } else { n - 1 };
                self.switch_tab(index);
            }
            "m" => {
                if let Some(r) = &self.renderer {
                    r.window().set_minimized(true);
                }
            }
            _ => return false,
        }
        true
    }

    /// Whether Option should send Meta for the current key press.
    fn option_as_meta(&self) -> bool {
        let (left, right) = self.option_keys;
        match self.config.option_as_meta {
            OptionAsMeta::None => false,
            OptionAsMeta::Both => self.mods.alt_key(),
            // Platforms without left/right reporting treat any Alt as left.
            OptionAsMeta::Left => left || (self.mods.alt_key() && !right),
        }
    }

    fn show_notice(&mut self, text: String, is_error: bool) {
        let duration = Duration::from_secs(if is_error { 12 } else { 3 });
        self.notice_until = Some(Instant::now() + duration);
        if let Some(r) = self.renderer.as_mut() {
            r.set_notice(Some(Notice { text, is_error }));
        }
        self.redraw();
    }

    /// Shows the light or dark variant, per the appearance setting, the
    /// system's mode and any Cmd+Shift+L flip.
    fn select_theme(&mut self) {
        let index = self.config.theme_index(&self.themes, self.system_dark, self.flip, None);
        if index != self.current {
            // The incoming theme takes over the window its way, a different
            // way each time, on a new sheet unless the settings pin one.
            let pinned = self.config.seed.is_some() || std::env::var_os("ONEPROMPT_SEED").is_some();
            let incoming = self.themes[index].clone();
            if let Some(r) = self.renderer.as_mut() {
                r.start_transition(&incoming);
                if !pinned {
                    r.new_sheet();
                }
            }
            self.current = index;
            self.relayout();
            self.check_font();
            // The panel shows the settings of the variant on screen.
            self.update_settings();
        }
    }

    /// Applies a config: themes with overrides, appearance, paper seed.
    /// When it changes which theme or variant is shown, the incoming one
    /// takes over the window its way.
    fn apply_config(&mut self, config: Config) {
        // A new appearance setting replaces a Cmd+Shift+L flip.
        if config.appearance != self.config.appearance {
            self.flip = None;
        }
        let shown = self.themes[self.current].name;
        self.themes = config.themes();
        // Snapshots must be deterministic, so they never animate.
        let (enabled, speed, writing_speed) = config.animation();
        // With input methods on, macOS would take Option+key as a dead key
        // (Option+E starts an accent); the Meta keys must reach the shell.
        #[cfg(target_os = "macos")]
        if let Some(r) = self.renderer.as_ref() {
            use winit::platform::macos::{OptionAsAlt, WindowExtMacOS};
            r.window().set_option_as_alt(match config.option_as_meta {
                OptionAsMeta::Left => OptionAsAlt::OnlyLeft,
                OptionAsMeta::Both => OptionAsAlt::Both,
                OptionAsMeta::None => OptionAsAlt::None,
            });
        }
        if let Some(r) = self.renderer.as_mut() {
            r.set_animation(enabled && self.snapshot_dir.is_none(), speed, writing_speed);
            r.set_harmonize(config.harmonize_colors.unwrap_or(true));
            r.set_splatter(config.strike_splatter.unwrap_or(true));
            r.set_command_marks(config.mark_failures.unwrap_or(true));
        }
        // ONEPROMPT_SEED (used for screenshots) wins over the file.
        if std::env::var_os("ONEPROMPT_SEED").is_none()
            && let Some(r) = self.renderer.as_mut()
        {
            r.set_seed(config.seed);
        }
        self.config = config;
        self.current = self.config.theme_index(&self.themes, self.system_dark, self.flip, None);
        if self.themes[self.current].name != shown {
            let pinned = self.config.seed.is_some() || std::env::var_os("ONEPROMPT_SEED").is_some();
            let incoming = self.themes[self.current].clone();
            if let Some(r) = self.renderer.as_mut() {
                r.start_transition(&incoming);
                if !pinned {
                    r.new_sheet();
                }
            }
        }
        self.relayout();
    }

    /// Warns (in a notice) when the current theme's font would break
    /// the grid: not installed, or not monospace. Returns true if it warned.
    fn check_font(&mut self) -> bool {
        let family = self.themes[self.current].font_family.clone();
        let problem = self.renderer.as_mut().and_then(|r| r.font_problem(&family));
        match problem {
            Some(problem) => {
                self.show_notice(problem, true);
                true
            }
            None => false,
        }
    }

    fn reload_config(&mut self) {
        let text = match std::fs::read_to_string(&self.config_path) {
            Ok(text) => Some(text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return self.show_notice(format!("settings not read: {err}"), true),
        };
        // The panel's own write, applied when it was made.
        if text.is_some() && text == self.written {
            return;
        }
        self.written = None;
        match text.as_deref().map_or(Ok(Config::default()), Config::parse) {
            // Saved, but nothing that counts changed: most likely an edit on a
            // line that is still commented out.
            Ok(config) if config == self.config && self.config_error.is_none() => self.show_notice(
                "settings saved, nothing changed (lines starting with # are ignored)".into(),
                false,
            ),
            Ok(config) => {
                self.config_error = None;
                self.apply_config(config);
                if !self.check_font() {
                    self.show_notice("settings applied".into(), false);
                }
            }
            Err(err) => {
                self.show_notice(format!("settings not applied: {err}"), true);
                self.config_error = Some(err);
            }
        }
        self.update_settings();
    }

    /// Input method events: text being composed is shown at the cursor;
    /// finished text goes where typing goes (a tab name being edited, else
    /// the shell).
    fn on_ime(&mut self, ime: winit::event::Ime) {
        use winit::event::Ime;
        let preedit = match ime {
            // Text composed for the find bar is shown in it.
            Ime::Preedit(text, _) if self.search.is_some() => {
                self.search_preedit = text;
                self.update_search(false);
                None
            }
            Ime::Preedit(text, cursor) if !text.is_empty() => Some((text, cursor)),
            Ime::Preedit(..) | Ime::Disabled | Ime::Enabled => None,
            Ime::Commit(text) => {
                if let Some(search) = self.search.as_ref() {
                    let query = search.query.clone() + &text;
                    self.search_preedit.clear();
                    self.set_search_query(query);
                } else if let Some(rename) = self.renaming.as_mut() {
                    if rename.all {
                        rename.text.clear();
                        rename.all = false;
                    }
                    rename.text.push_str(&text);
                    self.update_tabs();
                } else if self.picker.is_none()
                    && self.settings.is_none()
                    && let Some(t) = self.terminal()
                {
                    t.clear_selection();
                    t.input(text.into_bytes());
                }
                None
            }
        };
        if let Some(r) = self.renderer.as_mut() {
            r.set_preedit(preedit);
            r.window().request_redraw();
        }
    }

    /// Shows the find bar and its matches as they are, finding them again
    /// first if the terminal has changed (at most five times a second while
    /// output pours in), and with `reveal`, scrolls to the current match.
    fn update_search(&mut self, reveal: bool) {
        let Some(search) = self.search.as_mut() else { return };
        let Some(tab) = self.tabs.get(self.active) else { return };
        let mut term = tab.pane().terminal.term.lock();
        if self.search_stale && self.searched_at.elapsed() >= Duration::from_millis(200) {
            search.refresh(&term);
            self.search_stale = false;
            self.searched_at = Instant::now();
        }
        if reveal {
            let delta = search.reveal(&term);
            if delta != 0 {
                term.scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
            }
        }
        let marks = search.marks(&term);
        drop(term);
        let bar = renderer::SearchBar {
            query: search.query.clone(),
            preedit: self.search_preedit.clone(),
            count: search.count(),
            regex: search.regex,
            invalid: search.invalid,
        };
        if let Some(r) = self.renderer.as_mut() {
            r.set_search(Some(bar));
            r.set_search_marks(marks);
            if self.search_stale {
                // Come back once the wait is over.
                r.window().request_redraw();
            }
        }
    }

    /// Changes what is being looked for.
    fn set_search_query(&mut self, query: String) {
        let (Some(search), Some(tab)) = (self.search.as_mut(), self.tabs.get(self.active)) else { return };
        search.set_query(&tab.pane().terminal.term.lock(), query);
        self.searched_at = Instant::now();
        self.search_stale = false;
        self.update_search(true);
    }

    /// Closes the find bar, leaving the current match selected (Cmd+C
    /// copies it).
    fn close_search(&mut self) {
        let Some(search) = self.search.take() else { return };
        self.search_preedit.clear();
        if let (Some(m), Some(tab)) = (search.current_match(), self.tabs.get(self.active)) {
            use alacritty_terminal::index::Side;
            let mut selection = alacritty_terminal::selection::Selection::new(SelectionType::Simple, *m.start(), Side::Left);
            selection.update(*m.end(), Side::Right);
            tab.pane().terminal.term.lock().selection = Some(selection);
        }
        if let Some(r) = self.renderer.as_mut() {
            r.set_search(None);
            r.set_search_marks(Vec::new());
            r.window().request_redraw();
        }
    }

    /// Keys while the find bar is open: typing edits what is looked for,
    /// Return and the arrows go between matches. Other Cmd shortcuts still
    /// work; nothing reaches the shell.
    fn search_key(&mut self, event: &KeyEvent) -> bool {
        use winit::keyboard::NamedKey;
        let Some(search) = self.search.as_mut() else { return false };
        let mut query = search.query.clone();
        let shift = self.mods.shift_key();
        if self.mods.super_key() {
            match &event.logical_key {
                Key::Character(c) if c.eq_ignore_ascii_case("g") => {
                    search.step(!shift);
                    self.update_search(true);
                }
                Key::Character(c) if c == "v" => {
                    if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        query.extend(text.chars().map(|c| if c.is_control() { ' ' } else { c }));
                        self.set_search_query(query);
                    }
                }
                Key::Character(c) if c == "f" => {}
                Key::Character(c) if c.eq_ignore_ascii_case("r") => {
                    if let Some(tab) = self.tabs.get(self.active) {
                        search.toggle_regex(&tab.pane().terminal.term.lock());
                    }
                    self.update_search(true);
                }
                Key::Named(NamedKey::Backspace) => self.set_search_query(String::new()),
                _ => return false,
            }
            self.redraw();
            return true;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.close_search(),
            Key::Named(NamedKey::Enter) => {
                search.step(!shift);
                self.update_search(true);
            }
            Key::Named(NamedKey::ArrowUp) => {
                search.step(true);
                self.update_search(true);
            }
            Key::Named(NamedKey::ArrowDown) => {
                search.step(false);
                self.update_search(true);
            }
            Key::Named(NamedKey::Backspace) => {
                if self.mods.alt_key() {
                    // A word, and the spaces before it.
                    let kept = query.trim_end().len();
                    let cut = query[..kept].rfind(char::is_whitespace).map_or(0, |i| i + 1);
                    query.truncate(cut);
                } else {
                    query.pop();
                }
                self.set_search_query(query);
            }
            _ => {
                if let Some(typed) = event.text.as_deref().filter(|t| !t.chars().any(char::is_control)) {
                    query.push_str(typed);
                    self.set_search_query(query);
                }
            }
        }
        self.redraw();
        true
    }

    /// Draws every pane of the active tab, with the dividers between them.
    fn draw_frame(&mut self) {
        if self.snapshot_dir.is_some() {
            return;
        }
        // Matches follow the text as it scrolls and changes.
        self.update_search(false);
        let theme = self.themes[self.current].clone();
        let strike = self.config.strike_deleted.unwrap_or(true);
        if let (Some(r), Some(tab)) = (self.renderer.as_mut(), self.tabs.get_mut(self.active)) {
            let window = r.window_rect();
            let rects = tab.layout.rects(window);
            let scale = r.window().scale_factor() as f32;
            // Dividers start below the title strip, which runs across.
            let strip = renderer::TITLE_STRIP * scale;
            let dividers: Vec<renderer::Divider> = tab
                .layout
                .dividers(window, scale)
                .into_iter()
                .map(|d| renderer::Divider {
                    id: d.id,
                    line: match d.line {
                        [x, y, w, h] if d.across && y < strip => [x, strip, w, h - (strip - y)],
                        line => line,
                    },
                    upright: d.across,
                })
                .collect();
            let focus = tab.focus;
            // Every pane's screen, held still while it is drawn.
            let mut held = Vec::new();
            for pane in tab.panes.iter_mut() {
                let Some(&(_, rect)) = rects.iter().find(|(id, _)| *id == pane.id) else { continue };
                let Pane { id, terminal, strikes, .. } = pane;
                held.push((*id, rect, terminal.term.lock(), strike.then_some(strikes)));
            }
            let mut views: Vec<renderer::PaneView<_>> = held
                .iter_mut()
                .map(|(id, rect, term, strikes)| renderer::PaneView {
                    id: *id,
                    term: &**term,
                    strikes: strikes.as_deref_mut(),
                    rect: *rect,
                    focused: *id == focus,
                })
                .collect();
            // In reading order, the focused pane last so the window's
            // marks are drawn with it.
            views.sort_by_key(|v| rects.iter().position(|(id, _)| *id == v.id));
            r.render(&theme, &mut views, &dividers);
            drop(views);
            drop(held);
            // The input method's candidates open by the cursor.
            let area = r.ime_area();
            if area != self.ime_area {
                self.ime_area = area;
                let [x, y, w, h] = area;
                r.window().set_ime_cursor_area(
                    winit::dpi::PhysicalPosition::new(x as f64, y as f64),
                    winit::dpi::PhysicalSize::new(w as f64, h as f64),
                );
            }
            // Keep drawing while ink is still being brushed in or a
            // reveal is running; vsync paces it, idle costs nothing.
            if r.is_animating() {
                r.window().request_redraw();
            }
        }
    }

    /// Finds the link under the pointer while Cmd is held, underlines it
    /// and shows the pointing hand; otherwise clears them.
    fn update_link(&mut self) {
        let cell = self.pointer_cell().map(|(col, line, _)| (col, line));
        let wanted = self.mods.super_key() && self.picker.is_none() && self.settings.is_none() && !self.in_strip();
        let cell = cell.filter(|_| wanted);
        if cell.is_some() && self.link.as_ref().map(|(_, at)| Some(*at)) == Some(cell) {
            return;
        }
        let found = cell.and_then(|(col, line)| {
            let tab = self.tabs.get(self.active)?;
            let term = tab.pane().terminal.term.lock();
            let offset = term.grid().display_offset() as i32;
            let point = alacritty_terminal::index::Point::new(
                alacritty_terminal::index::Line(line as i32 - offset),
                alacritty_terminal::index::Column(col),
            );
            let link = links::at(&term, point, tab.pane().strikes.cwd().as_deref())?;
            Some((link, (col, line), offset))
        });
        let (underline, cursor) = match &found {
            Some((link, _, offset)) => (
                link.spans
                    .iter()
                    .filter_map(|&(line, col, len)| usize::try_from(line + offset).ok().map(|l| (l, col, len)))
                    .collect(),
                winit::window::CursorIcon::Pointer,
            ),
            None => (Vec::new(), winit::window::CursorIcon::Default),
        };
        let changed = found.is_some() || self.link.is_some();
        self.link = found.map(|(link, at, _)| (link, at));
        if changed && let Some(r) = self.renderer.as_mut() {
            r.set_link(underline);
            r.window().set_cursor(cursor);
            r.window().request_redraw();
        }
    }

    /// xterm modifier bits for mouse reports.
    fn mouse_mods(&self) -> u8 {
        4 * self.mods.shift_key() as u8
            + 8 * self.mods.alt_key() as u8
            + 16 * self.mods.control_key() as u8
    }

    /// The pointer is over the title strip.
    fn in_strip(&self) -> bool {
        let scale = self.renderer.as_ref().map_or(1.0, |r| r.window().scale_factor());
        self.mouse.y < renderer::TITLE_STRIP as f64 * scale
    }

    /// The cell of the focused pane under the pointer (clamped to its grid).
    fn pointer_cell(&self) -> Option<(usize, usize, bool)> {
        let (r, t, rect) = (self.renderer.as_ref()?, self.terminal()?, self.focus_rect()?);
        Some(r.cell_at(rect, self.mouse.x, self.mouse.y, t.size()))
    }

    /// The active tab's pane under the pointer.
    fn pane_under_pointer(&self) -> Option<PaneId> {
        let (x, y) = (self.mouse.x as f32, self.mouse.y as f32);
        self.pane_rects()
            .into_iter()
            .find(|(_, [rx, ry, rw, rh])| x >= *rx && x < rx + rw && y >= *ry && y < ry + rh)
            .map(|(id, _)| id)
    }

    /// The divider between panes under the pointer.
    fn divider_under_pointer(&self) -> Option<panes::Divider> {
        let (r, tab) = (self.renderer.as_ref()?, self.tabs.get(self.active)?);
        let grab = DIVIDER_GRAB * r.window().scale_factor() as f32;
        let (x, y) = (self.mouse.x as f32, self.mouse.y as f32);
        tab.layout
            .dividers(r.window_rect(), grab)
            .into_iter()
            .find(|d| x >= d.line[0] && x < d.line[0] + d.line[2] && y >= d.line[1] && y < d.line[1] + d.line[3])
    }

    /// Clicks on the theme-drawn window controls and the drag strip.
    /// Returns true if the event was handled here.
    fn on_chrome_button(&mut self, pressed: bool, button: MouseButton) -> bool {
        let Some(r) = &self.renderer else { return false };
        let window = r.window();
        let scale = window.scale_factor() as f32;
        let control = chrome::hit(self.mouse.x, self.mouse.y, scale);
        if button != MouseButton::Left {
            return false;
        }
        if !pressed {
            let Some(down) = self.mouse.pressed_control.take() else { return false };
            if control == Some(down) {
                match down {
                    Control::Close => self.close_requested = true,
                    Control::Minimize => window.set_minimized(true),
                    Control::Zoom => window.set_maximized(!window.is_maximized()),
                }
            }
            return true;
        }
        if control.is_some() {
            self.mouse.pressed_control = control;
            return true;
        }
        if self.mouse.y < renderer::TITLE_STRIP as f64 * scale as f64 {
            if let Some(index) = r.tab_at(self.mouse.x) {
                let now = Instant::now();
                let double = self.mouse.last_tab_click.is_some_and(|(at, i)| i == index && now - at < MULTI_CLICK);
                self.mouse.last_tab_click = Some((now, index));
                if self.renaming.as_ref().is_some_and(|r| r.index == index) {
                    return true;
                }
                self.switch_tab(index);
                if double {
                    self.start_rename(index);
                }
                return true;
            }
            let now = Instant::now();
            if self.mouse.last_strip_click.is_some_and(|at| now - at < MULTI_CLICK) {
                window.set_maximized(!window.is_maximized());
                self.mouse.last_strip_click = None;
            } else {
                self.mouse.last_strip_click = Some(now);
                let _ = window.drag_window();
            }
            return true;
        }
        false
    }

    fn on_mouse_button(&mut self, state: ElementState, button: MouseButton) {
        let pressed = state == ElementState::Pressed;
        if self.settings.is_some() {
            if button == MouseButton::Left {
                self.settings_button(pressed);
            }
            return;
        }
        // A click on a card applies it; anywhere else closes the picker.
        if self.picker.is_some() {
            if pressed && button == MouseButton::Left {
                match self.picker_card() {
                    Some(card) => self.apply_picked(card),
                    None => self.close_picker(),
                }
            }
            return;
        }
        // Cmd+click opens the link under the pointer, even in programs
        // that take the mouse.
        if pressed && button == MouseButton::Left && self.mods.super_key() {
            self.update_link();
            if let Some((link, _)) = &self.link {
                if let Err(err) = links::open(&link.target) {
                    self.show_notice(format!("could not open the link: {err}"), true);
                }
                return;
            }
        }
        // A click anywhere else keeps a name being edited.
        if pressed && self.renaming.is_some() {
            let on_tab = self.renderer.as_ref().and_then(|r| r.tab_at(self.mouse.x));
            if !self.in_strip() || on_tab != self.renaming.as_ref().map(|r| r.index) {
                self.finish_rename(true);
            }
        }
        if self.on_chrome_button(pressed, button) {
            return;
        }
        // Dividers between panes are dragged; a click in a pane gives it
        // the keyboard, and then acts in it as usual.
        if button == MouseButton::Left && !pressed && self.mouse.divider.take().is_some() {
            return;
        }
        if pressed {
            if button == MouseButton::Left
                && let Some(divider) = self.divider_under_pointer()
            {
                self.mouse.divider = Some(divider);
                return;
            }
            if let Some(id) = self.pane_under_pointer()
                && self.tabs.get(self.active).is_some_and(|t| t.focus != id)
            {
                self.focus_pane(id);
            }
        }
        // Right-click opens the menu, unless the program wants the mouse
        // (Shift+right-click still opens it then). On a tab, for that tab.
        if button == MouseButton::Right {
            let program_wants = self.terminal().is_some_and(|t| t.mouse_mode()) && !self.mods.shift_key();
            if self.in_strip() || !program_wants {
                if pressed {
                    if let Some(index) = self.renderer.as_ref().and_then(|r| r.tab_at(self.mouse.x)).filter(|_| self.in_strip()) {
                        self.switch_tab(index);
                    }
                    self.show_menu();
                }
                return;
            }
        }
        let Some((col, line, right_half)) = self.pointer_cell() else { return };
        let Some(t) = self.tabs.get(self.active).map(|t| &t.pane().terminal) else { return };

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
            // A click without a drag selects nothing: drop it, so it does
            // not linger as a one-cell selection.
            self.mouse.selecting = false;
            t.drop_empty_selection();
            self.redraw();
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
        if self.settings.is_some() {
            self.settings_drag();
            return;
        }
        // Over the picker, the card under the pointer is the chosen one.
        if self.picker.is_some() {
            if let Some(card) = self.picker_card()
                && self.picker != Some(card)
            {
                self.picker = Some(card);
                self.update_picker();
            }
            return;
        }
        if let Some(divider) = self.mouse.divider.clone() {
            let [x, y, w, h] = divider.split;
            let ratio = if divider.across { (self.mouse.x as f32 - x) / w } else { (self.mouse.y as f32 - y) / h };
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.layout.set_ratio(&divider.path, ratio);
            }
            self.relayout();
            return;
        }
        self.update_link();
        // Over a divider, the pointer says it can be dragged.
        if self.link.is_none()
            && let Some(r) = self.renderer.as_ref()
        {
            use winit::window::CursorIcon;
            let icon = match self.divider_under_pointer() {
                Some(d) if d.across => CursorIcon::ColResize,
                Some(_) => CursorIcon::RowResize,
                None => CursorIcon::Default,
            };
            r.window().set_cursor(icon);
        }
        if let Some(r) = self.renderer.as_mut() {
            let over = chrome::over_group(self.mouse.x, self.mouse.y, r.window().scale_factor() as f32);
            if r.set_controls_hover(over) {
                r.window().request_redraw();
            }
        }
        let Some((col, line, right_half)) = self.pointer_cell() else { return };
        let Some(t) = self.tabs.get(self.active).map(|t| &t.pane().terminal) else { return };
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
            if let (Some(r), Some(t)) = (self.renderer.as_mut(), self.tabs.get(self.active).map(|t| &t.pane().terminal)) {
                let term = t.term.lock();
                r.snapshot(&theme, &term, &dir.join(format!("{name}.png")));
            }
        }
        // Frames of the switch into each theme from the one before it.
        let frames = std::env::var("ONEPROMPT_SNAPSHOT_SWITCH").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
        if frames == 0 {
            return;
        }
        for i in 0..self.themes.len() {
            let from = self.themes[(i + self.themes.len() - 1) % self.themes.len()].clone();
            self.current = i;
            self.relayout();
            let to = self.themes[i].clone();
            let name = to.name.to_lowercase().replace(' ', "-");
            if let (Some(r), Some(t)) = (self.renderer.as_mut(), self.tabs.get(self.active).map(|t| &t.pane().terminal)) {
                let term = t.term.lock();
                r.snapshot_switch(&from, &to, &term, frames, &dir.join(format!("to-{name}")));
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
        let attrs = match self.position {
            Some((x, y)) => attrs.with_position(winit::dpi::LogicalPosition::new(x, y)),
            None => attrs,
        };
        // No native title bar or buttons: the surface runs to the top edge and
        // the theme draws its own controls (chrome.rs). The top strip drags
        // the window and double-click zooms it.
        #[cfg(target_os = "macos")]
        let attrs = {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs.with_titlebar_hidden(true)
        };
        #[cfg(not(target_os = "macos"))]
        let attrs = attrs.with_decorations(false);
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        // Input methods: composed text (Chinese, Japanese, Korean, accents
        // typed as two keys) arrives as Ime events.
        window.set_ime_allowed(true);
        let mut renderer = Renderer::new(window);
        let id = self.next_pane;
        self.next_pane += 1;
        let window = renderer.window_rect();
        let size = renderer.grid_size(self.theme(), id, window);
        let integrate = self.config.shell_integration.unwrap_or(true);
        match Terminal::spawn(self.proxy.clone(), id, size, self.command.clone(), None, integrate) {
            Ok(terminal) => self.tabs.push(Tab::new(Pane::new(id, terminal))),
            Err(err) => {
                eprintln!("oneprompt: could not start shell: {err}");
                event_loop.exit();
                return;
            }
        }
        renderer.window().request_redraw();
        self.renderer = Some(renderer);
        self.menu = Some(menu::ContextMenu::new(self.proxy.clone()));
        self.update_tabs();
        // Seed, notices and the system's light/dark mode need the window.
        self.system_dark = self.renderer.as_ref().and_then(|r| r.window().theme())
            == Some(winit::window::Theme::Dark);
        let config = self.config.clone();
        self.apply_config(config);
        // The window opens out of its opposite: a light look is uncovered
        // from its ink colour, a dark one floods over its theme's light
        // surface.
        let variant = self.themes[self.current].clone();
        let from = if variant.dark {
            self.themes
                .iter()
                .find(|v| !v.dark && v.theme.id() == variant.theme.id())
                .map_or(variant.ink, |v| v.surface_rgb())
        } else {
            variant.ink
        };
        if let Some(r) = self.renderer.as_mut() {
            r.start_intro(from, &variant);
        }
        if let Some(err) = self.startup_error.take() {
            self.show_notice(format!("settings not applied: {err}"), true);
            self.config_error = Some(err);
        } else {
            self.check_font();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Sleep until the notice should go away, then clear it.
        match self.notice_until {
            Some(until) if Instant::now() >= until => {
                self.notice_until = None;
                if let Some(r) = self.renderer.as_mut() {
                    r.set_notice(None);
                }
                self.redraw();
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(until) => event_loop.set_control_flow(ControlFlow::WaitUntil(until)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            // Background tabs keep running; only the visible one is drawn.
            UserEvent::Wakeup(id) if self.find_pane(id).is_some_and(|(t, _)| t == self.active) => {
                self.search_stale = true;
                // Full-screen apps get room for at least 80x24 cells, in
                // whichever pane they run.
                let full_screen = self
                    .find_pane(id)
                    .is_some_and(|(t, p)| self.tabs[t].panes[p].terminal.mode().contains(TermMode::ALT_SCREEN));
                if self.renderer.as_mut().is_some_and(|r| r.set_full_screen(id, full_screen)) {
                    self.relayout();
                }
                self.redraw();
            }
            UserEvent::Wakeup(_) => {}
            UserEvent::Title(id, title) => {
                if let Some((t, p)) = self.find_pane(id) {
                    self.tabs[t].panes[p].title = title;
                    self.update_tabs();
                }
            }
            UserEvent::PtyWrite(id, text) => {
                if let Some((t, p)) = self.find_pane(id) {
                    self.tabs[t].panes[p].terminal.write(text.into_bytes());
                }
            }
            // In snapshot mode the command may finish before we capture.
            UserEvent::Exit(_) if self.snapshot_dir.is_some() => {}
            // A shell that exits closes its pane, and the tab with its last.
            UserEvent::Exit(id) => match self.find_pane(id) {
                Some((t, _)) if self.tabs[t].panes.len() > 1 => self.remove_pane(t, id),
                Some((t, _)) => {
                    if !self.close_tab(t) {
                        event_loop.exit();
                    }
                }
                None => {}
            },
            UserEvent::ConfigChanged => self.reload_config(),
            UserEvent::Command(id, n, finished) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.finish_command(id, n, finished);
                }
                if self.find_pane(id).is_some_and(|(t, _)| t == self.active) {
                    self.redraw();
                }
            }
            UserEvent::Menu(action) => {
                self.menu_action(action);
                if self.close_requested {
                    event_loop.exit();
                }
            }
            // Tell programs the theme's real colours, so the ones that check
            // (to choose a light or dark palette) see the sheet they are on.
            UserEvent::ColorRequest(id, index, reply) => {
                let [r, g, b] = renderer::query_color(&self.themes[self.current], index);
                if let Some((t, p)) = self.find_pane(id) {
                    self.tabs[t].panes[p].terminal.write(reply(alacritty_terminal::vte::ansi::Rgb { r, g, b }).into_bytes());
                }
            }
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
                // Drawn now, not on the next redraw, so the frame for the
                // new size goes out with the resize itself.
                self.draw_frame();
            }
            WindowEvent::ScaleFactorChanged { .. } => self.relayout(),
            WindowEvent::ThemeChanged(theme) => {
                self.system_dark = theme == winit::window::Theme::Dark;
                self.select_theme();
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.mods = mods.state();
                self.update_link();
                self.option_keys = (
                    mods.lalt_state() == ModifiersKeyState::Pressed,
                    mods.ralt_state() == ModifiersKeyState::Pressed,
                );
            }
            WindowEvent::Focused(focused) => {
                if let Some(t) = self.terminal() {
                    t.set_focused(focused);
                }
                self.redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.renaming.is_some() && self.rename_key(&event) {
                    return;
                }
                if self.picker_key(&event) || self.settings_key(&event) || self.search_key(&event) {
                    return;
                }
                if self.shortcut(&event) {
                    if self.close_requested {
                        event_loop.exit();
                    }
                    return;
                }
                let Some(t) = self.terminal() else { return };
                let app_cursor = t.mode().contains(TermMode::APP_CURSOR);
                let meta = self.option_as_meta();
                let bytes = input::key_to_bytes(&event, self.mods, meta, app_cursor);
                if let Some(bytes) = bytes {
                    t.clear_selection();
                    t.input(bytes);
                }
            }
            WindowEvent::Ime(ime) => self.on_ime(ime),
            WindowEvent::CursorMoved { position, .. } => {
                (self.mouse.x, self.mouse.y) = (position.x, position.y);
                self.on_mouse_move();
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(r) = self.renderer.as_mut()
                    && r.set_controls_hover(false)
                {
                    r.window().request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.on_mouse_button(state, button);
                if self.close_requested {
                    event_loop.exit();
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.settings.is_some() => {
                let Some(r) = self.renderer.as_mut() else { return };
                let pixels = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 3.0 * r.cell_height(),
                    MouseScrollDelta::PixelDelta(p) => -p.y as f32,
                };
                if r.settings_contains(self.mouse.x as f32, self.mouse.y as f32) {
                    r.scroll_settings(pixels);
                    r.window().request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // The pane under the pointer scrolls, keyboard or not.
                let under = self.pane_under_pointer();
                let Some(tab) = self.tabs.get(self.active) else { return };
                let focused = under.is_none_or(|id| id == tab.focus);
                let pane = under.and_then(|id| tab.panes.iter().find(|p| p.id == id)).unwrap_or(tab.pane());
                let (Some(r), t) = (&self.renderer, &pane.terminal) else {
                    return;
                };
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
                if t.mouse_mode() && lines != 0 && focused {
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
                self.draw_frame();
            }
            _ => {}
        }
    }
}

/// A tab's name in the strip: what its program called it, else the shell's.
fn tab_label(tab: &Tab, index: usize) -> String {
    if let Some(name) = &tab.name {
        return name.clone();
    }
    if let Some(title) = tab.title().map(str::trim).filter(|t| !t.is_empty()) {
        return title.to_string();
    }
    let shell = std::env::var("SHELL").ok();
    let name = shell.as_deref().and_then(|s| s.rsplit('/').next()).unwrap_or("shell");
    format!("{name} {}", index + 1)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut command = None;
    let mut position = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-e" => command = args.next(),
            "--position" => {
                position = args.next().and_then(|p| {
                    let (x, y) = p.split_once(',')?;
                    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
                })
            }
            "--default-config" => {
                print!("{}", config::template());
                return;
            }
            "-h" | "--help" => {
                println!("usage: oneprompt [-e COMMAND] [--position X,Y] [--default-config]");
                println!("settings: {}", config::path().display());
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
    let config_path = config::path();
    let (config, startup_error) = match Config::load(&config_path) {
        Ok(config) => (config, None),
        Err(err) => (Config::default(), Some(err)),
    };
    config::watch(config_path.clone(), event_loop.create_proxy());
    let themes = config.themes();
    // The window (and with it the system's light/dark mode) does not exist
    // yet; `resumed` picks the right variant.
    let current = config.theme_index(&themes, false, None, None);
    let mut app = App {
        proxy: event_loop.create_proxy(),
        command,
        snapshot_dir: std::env::var_os("ONEPROMPT_SNAPSHOT").map(PathBuf::from),
        started: Instant::now(),
        position,
        renderer: None,
        tabs: Vec::new(),
        active: 0,
        next_pane: 0,
        renaming: None,
        menu: None,
        themes,
        current,
        picker: None,
        settings: None,
        written: None,
        config_error: None,
        ime_area: [0.0; 4],
        link: None,
        search: None,
        search_preedit: String::new(),
        search_stale: false,
        searched_at: Instant::now(),
        mods: ModifiersState::empty(),
        option_keys: (false, false),
        config_path,
        config,
        notice_until: None,
        system_dark: false,
        flip: None,
        startup_error,
        scroll_px: 0.0,
        mouse: Mouse::default(),
        close_requested: false,
    };
    event_loop.run_app(&mut app).expect("run event loop");
}
