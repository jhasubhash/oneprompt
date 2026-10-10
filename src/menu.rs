//! The right-click menu: the system's own (muda), so it looks and behaves
//! like every other Mac app's. Choosing an item arrives as `UserEvent::Menu`
//! and `App::menu_action` carries it out, the same code the shortcuts use.

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{ContextMenu as _, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use winit::event_loop::EventLoopProxy;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::terminal::UserEvent;

/// What a menu item does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Copy,
    Paste,
    SelectAll,
    Clear,
    NewTab,
    NewWindow,
    SplitRight,
    SplitDown,
    RenameTab,
    CloseTab,
    Bigger,
    Smaller,
    ActualSize,
    FlipAppearance,
    Themes,
    Settings,
    SettingsFile,
    Find,
}

impl Action {
    const ALL: [Action; 18] = [
        Action::Copy,
        Action::Paste,
        Action::SelectAll,
        Action::Clear,
        Action::NewTab,
        Action::NewWindow,
        Action::SplitRight,
        Action::SplitDown,
        Action::RenameTab,
        Action::CloseTab,
        Action::Bigger,
        Action::Smaller,
        Action::ActualSize,
        Action::FlipAppearance,
        Action::Themes,
        Action::Settings,
        Action::SettingsFile,
        Action::Find,
    ];

    fn id(self) -> String {
        format!("{self:?}")
    }

    fn from_id(id: &str) -> Option<Action> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    /// The action named `name` in kebab case ("clear", "new-tab",
    /// "split-right", ...), for `ONEPROMPT_ACTIONS`.
    pub fn from_name(name: &str) -> Option<Action> {
        let flat: String = name.chars().filter(|c| *c != '-' && *c != '_').collect();
        Self::ALL.into_iter().find(|a| a.id().eq_ignore_ascii_case(&flat))
    }
}

/// What the menu should say right now.
pub struct State {
    pub has_selection: bool,
    pub dark: bool,
    pub tabs: usize,
    /// Panes in the active tab.
    pub panes: usize,
}

pub struct ContextMenu {
    menu: Menu,
    copy: MenuItem,
    flip: MenuItem,
    close: MenuItem,
}

impl ContextMenu {
    /// Builds the menu and routes its clicks to the event loop.
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(action) = Action::from_id(event.id.as_ref()) {
                let _ = proxy.send_event(UserEvent::Menu(action));
            }
        }));
        let cmd = |key| Some(Accelerator::new(Modifiers::META, key));
        let cmd_shift = |key| Some(Accelerator::new(Modifiers::META | Modifiers::SHIFT, key));
        let item = |action: Action, text: &str, accel| MenuItem::with_id(action.id(), text, true, accel);

        let copy = item(Action::Copy, "Copy", cmd(Code::KeyC));
        let flip = item(Action::FlipAppearance, "Switch to Dark", cmd_shift(Code::KeyL));
        let close = item(Action::CloseTab, "Close Tab", cmd(Code::KeyW));
        let menu = Menu::new();
        let _ = menu.append_items(&[
            &copy,
            &item(Action::Paste, "Paste", cmd(Code::KeyV)),
            &item(Action::SelectAll, "Select All", cmd(Code::KeyA)),
            &item(Action::Clear, "Clear", cmd(Code::KeyK)),
            &item(Action::Find, "Find…", cmd(Code::KeyF)),
            &PredefinedMenuItem::separator(),
            &item(Action::NewTab, "New Tab", cmd(Code::KeyT)),
            &item(Action::NewWindow, "New Window", cmd(Code::KeyN)),
            &item(Action::SplitRight, "Split Right", cmd(Code::KeyD)),
            &item(Action::SplitDown, "Split Down", cmd_shift(Code::KeyD)),
            &item(Action::RenameTab, "Rename Tab…", None),
            &close,
            &PredefinedMenuItem::separator(),
            &item(Action::Bigger, "Bigger Text", cmd(Code::Equal)),
            &item(Action::Smaller, "Smaller Text", cmd(Code::Minus)),
            &item(Action::ActualSize, "Actual Size", cmd(Code::Digit0)),
            &flip,
            &item(Action::Themes, "Themes…", Some(Accelerator::new(Modifiers::META | Modifiers::ALT, Code::KeyT))),
            &PredefinedMenuItem::separator(),
            &item(Action::Settings, "Settings…", cmd(Code::Comma)),
            &item(Action::SettingsFile, "Edit Settings File…", None),
        ]);
        Self { menu, copy, flip, close }
    }

    /// Shows the menu at the pointer. Returns once it is dismissed; the
    /// chosen item (if any) arrives as an event.
    pub fn show(&self, window: &Window, state: State) {
        self.copy.set_enabled(state.has_selection);
        self.flip.set_text(if state.dark { "Switch to Light" } else { "Switch to Dark" });
        self.close.set_text(if state.panes > 1 {
            "Close Pane"
        } else if state.tabs > 1 {
            "Close Tab"
        } else {
            "Close Window"
        });
        let Ok(handle) = window.window_handle() else { return };
        #[cfg(target_os = "macos")]
        if let RawWindowHandle::AppKit(handle) = handle.as_raw() {
            // SAFETY: the view belongs to `window`, which outlives this call.
            unsafe {
                self.menu.show_context_menu_for_nsview(handle.ns_view.as_ptr(), None);
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = handle;
    }
}
