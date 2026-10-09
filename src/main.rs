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

mod animation;
mod boxdraw;
mod chrome;
mod color;
mod config;
mod input;
mod renderer;
mod terminal;
mod theme;
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
use renderer::{Notice, Renderer};
use terminal::{TabId, Terminal, UserEvent};
use theme::Theme;

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
    next_tab: TabId,
    themes: Vec<Theme>,
    current: usize,
    mods: ModifiersState,
    /// Which Option keys are held right now: (left, right).
    option_keys: (bool, bool),
    config_path: PathBuf,
    config: Config,
    /// When the title-strip notice should disappear.
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

struct Tab {
    id: TabId,
    terminal: Terminal,
    /// Set by the program running in it (OSC 0/2).
    title: Option<String>,
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
    /// Window control pressed; it acts on release over the same control.
    pressed_control: Option<Control>,
    /// Last press in the title strip, for double-click to zoom.
    last_strip_click: Option<Instant>,
}

/// Clicks closer together than this count as a double/triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);

impl App {
    fn theme(&self) -> &Theme {
        &self.themes[self.current]
    }

    /// The active tab's terminal.
    fn terminal(&self) -> Option<&Terminal> {
        self.tabs.get(self.active).map(|t| &t.terminal)
    }

    /// Recomputes the grid for the window and theme, and tells the shells.
    /// Every tab gets the window's size, so switching never resizes.
    fn relayout(&mut self) {
        let theme = self.themes[self.current].clone();
        let Some(renderer) = self.renderer.as_mut() else { return };
        let size = renderer.grid_size(&theme);
        for tab in &mut self.tabs {
            tab.terminal.resize(size);
        }
        renderer.window().request_redraw();
    }

    /// Starts a shell in a new tab and shows it.
    fn new_tab(&mut self) -> std::io::Result<()> {
        let theme = self.themes[self.current].clone();
        let Some(renderer) = self.renderer.as_mut() else { return Ok(()) };
        let id = self.next_tab;
        self.next_tab += 1;
        let terminal = Terminal::spawn(self.proxy.clone(), id, renderer.grid_size(&theme), self.command.clone())?;
        self.tabs.push(Tab { id, terminal, title: None });
        self.switch_tab(self.tabs.len() - 1);
        Ok(())
    }

    /// Shows tab `index`.
    fn switch_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if let Some(old) = self.tabs.get(self.active).filter(|_| self.active != index) {
            old.terminal.set_focused(false);
        }
        self.active = index;
        self.mouse.selecting = false;
        self.mouse.reported_button = None;
        self.mouse.reported_cell = None;
        let focused = self.renderer.as_ref().is_some_and(|r| r.window().has_focus());
        self.tabs[index].terminal.set_focused(focused);
        if let Some(r) = self.renderer.as_mut() {
            // Another tab's screen is not new ink: show it as it stands.
            r.reset_writing();
        }
        self.update_tabs();
        // A full-screen app in the new tab may need its text shrunk.
        let full_screen = self.terminal().is_some_and(|t| t.mode().contains(TermMode::ALT_SCREEN));
        if let Some(r) = self.renderer.as_mut() {
            r.set_full_screen(full_screen);
        }
        self.relayout();
    }

    /// Closes tab `index`; returns false when it was the last one.
    fn close_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return true;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            return false;
        }
        let active = if self.active > index || self.active >= self.tabs.len() { self.active - 1 } else { self.active };
        self.active = usize::MAX;
        self.switch_tab(active.min(self.tabs.len() - 1));
        true
    }

    /// Tells the renderer the tab labels and sets the window title.
    fn update_tabs(&mut self) {
        let labels: Vec<String> = self.tabs.iter().enumerate().map(|(i, t)| tab_label(t, i)).collect();
        if let Some(r) = self.renderer.as_mut() {
            if let Some(label) = labels.get(self.active) {
                r.window().set_title(label);
            }
            r.set_tabs(if labels.len() > 1 { labels } else { Vec::new() }, self.active);
            r.window().request_redraw();
        }
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
    fn shortcut(&mut self, event: &KeyEvent) -> bool {
        if !self.mods.super_key() {
            return false;
        }
        let Key::Character(key) = &event.logical_key else {
            return false;
        };
        let shift = self.mods.shift_key();
        match key.as_str() {
            // Flip between the light and dark variant for this session.
            "l" | "L" if shift => {
                self.flip = Some(!self.themes[self.current].dark);
                self.select_theme();
            }
            "t" => {
                if let Err(err) = self.new_tab() {
                    self.show_notice(format!("could not open a tab: {err}"), true);
                }
            }
            "n" => self.new_window(),
            // Next / previous tab (Shift+] and Shift+[ arrive as braces).
            "}" | "]" if shift => {
                let n = self.tabs.len();
                self.switch_tab((self.active + 1) % n.max(1));
            }
            "{" | "[" if shift => {
                let n = self.tabs.len();
                self.switch_tab((self.active + n.max(1) - 1) % n.max(1));
            }
            // Cmd+1..8 pick a tab, Cmd+9 the last one.
            digit @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9") => {
                let n: usize = digit.parse().unwrap_or(1);
                let index = if n == 9 { self.tabs.len().saturating_sub(1) } else { n - 1 };
                self.switch_tab(index);
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
            "," => {
                if let Err(err) = config::open_in_editor(&self.config_path) {
                    self.show_notice(format!("could not open settings: {err}"), true);
                }
            }
            "w" => {
                if !self.close_tab(self.active) {
                    self.close_requested = true;
                }
            }
            "m" => {
                if let Some(r) = &self.renderer {
                    r.window().set_minimized(true);
                }
            }
            "c" => {
                if let Some(text) = self.terminal().and_then(|t| t.selection_text()) {
                    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
                }
            }
            "v" => {
                let text = arboard::Clipboard::new().and_then(|mut c| c.get_text());
                if let (Ok(text), Some(t)) = (text, self.terminal()) {
                    t.paste(&text);
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
        let index = self.config.theme_index(&self.themes, self.system_dark, self.flip);
        if index != self.current {
            // Brush the new look in over the old one, a different way each time.
            if let Some(r) = self.renderer.as_mut() {
                r.start_transition();
            }
            self.current = index;
            self.relayout();
            self.check_font();
        }
    }

    /// Applies a config: themes with overrides, appearance, paper seed.
    fn apply_config(&mut self, config: Config) {
        // A new appearance setting replaces a Cmd+Shift+L flip.
        if config.appearance != self.config.appearance {
            self.flip = None;
        }
        self.themes = config.themes();
        // Snapshots must be deterministic, so they never animate.
        let (enabled, speed, writing_speed) = config.animation();
        if let Some(r) = self.renderer.as_mut() {
            r.set_animation(enabled && self.snapshot_dir.is_none(), speed, writing_speed);
            r.set_harmonize(config.harmonize_colors.unwrap_or(true));
        }
        // ONEPROMPT_SEED (used for screenshots) wins over the file.
        if std::env::var_os("ONEPROMPT_SEED").is_none()
            && let Some(r) = self.renderer.as_mut()
        {
            r.set_seed(config.seed);
        }
        self.config = config;
        self.current = self.config.theme_index(&self.themes, self.system_dark, self.flip);
        self.relayout();
    }

    /// Warns (in the title strip) when the current theme's font would break
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
        match Config::load(&self.config_path) {
            // Saved, but nothing that counts changed: most likely an edit on a
            // line that is still commented out.
            Ok(config) if config == self.config => self.show_notice(
                "settings saved, nothing changed (lines starting with # are ignored)".into(),
                false,
            ),
            Ok(config) => {
                self.apply_config(config);
                if !self.check_font() {
                    self.show_notice("settings applied".into(), false);
                }
            }
            Err(err) => self.show_notice(format!("settings not applied: {err}"), true),
        }
    }

    /// xterm modifier bits for mouse reports.
    fn mouse_mods(&self) -> u8 {
        4 * self.mods.shift_key() as u8
            + 8 * self.mods.alt_key() as u8
            + 16 * self.mods.control_key() as u8
    }

    fn pointer_cell(&self) -> Option<(usize, usize, bool)> {
        let (r, t) = (self.renderer.as_ref()?, self.terminal()?);
        Some(r.cell_at(self.mouse.x, self.mouse.y, t.size()))
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
                self.switch_tab(index);
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
        if self.on_chrome_button(pressed, button) {
            return;
        }
        let Some((col, line, right_half)) = self.pointer_cell() else { return };
        let Some(t) = self.tabs.get(self.active).map(|t| &t.terminal) else { return };

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
        if let Some(r) = self.renderer.as_mut() {
            let over = chrome::over_group(self.mouse.x, self.mouse.y, r.window().scale_factor() as f32);
            if r.set_controls_hover(over) {
                r.window().request_redraw();
            }
        }
        let Some((col, line, right_half)) = self.pointer_cell() else { return };
        let Some(t) = self.tabs.get(self.active).map(|t| &t.terminal) else { return };
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
            if let (Some(r), Some(t)) = (self.renderer.as_mut(), self.tabs.get(self.active).map(|t| &t.terminal)) {
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
        let mut renderer = Renderer::new(window);
        let size = renderer.grid_size(self.theme());
        let id = self.next_tab;
        self.next_tab += 1;
        match Terminal::spawn(self.proxy.clone(), id, size, self.command.clone()) {
            Ok(terminal) => self.tabs.push(Tab { id, terminal, title: None }),
            Err(err) => {
                eprintln!("oneprompt: could not start shell: {err}");
                event_loop.exit();
                return;
            }
        }
        renderer.window().request_redraw();
        self.renderer = Some(renderer);
        self.update_tabs();
        // Seed, notices and the system's light/dark mode need the window.
        self.system_dark = self.renderer.as_ref().and_then(|r| r.window().theme())
            == Some(winit::window::Theme::Dark);
        let config = self.config.clone();
        self.apply_config(config);
        // Open the window like the ink paintings it imitates: light paper is
        // uncovered from a sheet of ink, dark ink floods over the paper.
        let theme = &self.themes[self.current];
        let from = if theme.dark {
            self.themes.iter().find(|t| !t.dark).map_or([226, 196, 142], |t| t.surface_rgb())
        } else {
            theme.ink
        };
        if let Some(r) = self.renderer.as_mut() {
            r.start_intro(from, [0.5, 1.0]);
        }
        if let Some(err) = self.startup_error.take() {
            self.show_notice(format!("settings not applied: {err}"), true);
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
        let tab = |id: TabId| self.tabs.iter().position(|t| t.id == id);
        match event {
            // Background tabs keep running; only the visible one is drawn.
            UserEvent::Wakeup(id) if tab(id) == Some(self.active) => {
                // Full-screen apps get room for at least 80x24 cells.
                let full_screen = self.terminal().is_some_and(|t| t.mode().contains(TermMode::ALT_SCREEN));
                if self.renderer.as_mut().is_some_and(|r| r.set_full_screen(full_screen)) {
                    self.relayout();
                }
                self.redraw();
            }
            UserEvent::Wakeup(_) => {}
            UserEvent::Title(id, title) => {
                if let Some(i) = tab(id) {
                    self.tabs[i].title = title;
                    self.update_tabs();
                }
            }
            UserEvent::PtyWrite(id, text) => {
                if let Some(i) = tab(id) {
                    self.tabs[i].terminal.write(text.into_bytes());
                }
            }
            // In snapshot mode the command may finish before we capture.
            UserEvent::Exit(_) if self.snapshot_dir.is_some() => {}
            UserEvent::Exit(id) => {
                if let Some(i) = tab(id)
                    && !self.close_tab(i)
                {
                    event_loop.exit();
                }
            }
            UserEvent::ConfigChanged => self.reload_config(),
            // Tell programs the theme's real colours, so the ones that check
            // (to choose a light or dark palette) see the sheet they are on.
            UserEvent::ColorRequest(id, index, reply) => {
                let [r, g, b] = renderer::query_color(&self.themes[self.current], index);
                if let Some(i) = tab(id) {
                    self.tabs[i].terminal.write(reply(alacritty_terminal::vte::ansi::Rgb { r, g, b }).into_bytes());
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
            }
            WindowEvent::ScaleFactorChanged { .. } => self.relayout(),
            WindowEvent::ThemeChanged(theme) => {
                self.system_dark = theme == winit::window::Theme::Dark;
                self.select_theme();
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.mods = mods.state();
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
            WindowEvent::MouseWheel { delta, .. } => {
                let (Some(r), Some(t)) = (&self.renderer, self.tabs.get(self.active).map(|t| &t.terminal)) else {
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
                if let (Some(r), Some(t)) = (self.renderer.as_mut(), self.tabs.get(self.active).map(|t| &t.terminal)) {
                    let term = t.term.lock();
                    r.render(&theme, &term);
                    drop(term);
                    // Keep drawing while ink is still being brushed in or a
                    // reveal is running; vsync paces it, idle costs nothing.
                    if r.is_animating() {
                        r.window().request_redraw();
                    }
                }
            }
            _ => {}
        }
    }
}

/// A tab's name in the strip: what its program called it, else the shell's.
fn tab_label(tab: &Tab, index: usize) -> String {
    if let Some(title) = tab.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
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
    let current = config.theme_index(&themes, false, None);
    let mut app = App {
        proxy: event_loop.create_proxy(),
        command,
        snapshot_dir: std::env::var_os("ONEPROMPT_SNAPSHOT").map(PathBuf::from),
        started: Instant::now(),
        position,
        renderer: None,
        tabs: Vec::new(),
        active: 0,
        next_tab: 0,
        themes,
        current,
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
