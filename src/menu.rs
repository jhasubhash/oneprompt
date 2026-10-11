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
    /// Opens or closes the file browser beside the focused pane.
    Browser,
    // The file browser's own menu, for its chosen entry.
    BrowserOpen,
    BrowserOpenApp,
    BrowserReveal,
    BrowserEdit,
    BrowserCd,
    BrowserInsert,
    BrowserCopyPath,
    BrowserCopyName,
    BrowserCopyRelative,
    BrowserNewFolder,
    BrowserNewFile,
    BrowserRename,
    BrowserTrash,
    BrowserHidden,
}

impl Action {
    const ALL: [Action; 33] = [
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
        Action::Browser,
        Action::BrowserOpen,
        Action::BrowserOpenApp,
        Action::BrowserReveal,
        Action::BrowserEdit,
        Action::BrowserCd,
        Action::BrowserInsert,
        Action::BrowserCopyPath,
        Action::BrowserCopyName,
        Action::BrowserCopyRelative,
        Action::BrowserNewFolder,
        Action::BrowserNewFile,
        Action::BrowserRename,
        Action::BrowserTrash,
        Action::BrowserHidden,
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
    /// The focused pane's file browser is open.
    pub browser: bool,
}

pub struct ContextMenu {
    menu: Menu,
    copy: MenuItem,
    flip: MenuItem,
    close: MenuItem,
    browser: MenuItem,
    /// The file browser's menu, for the entry right-clicked.
    entry: Menu,
    entry_items: Vec<MenuItem>,
    hidden: MenuItem,
}

/// What the file browser's menu should say.
pub struct EntryState {
    /// An entry is chosen (else only the folder's own items apply).
    pub has_entry: bool,
    pub folder: bool,
    pub show_hidden: bool,
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
        let browser = item(Action::Browser, "Show File Browser", cmd(Code::KeyB));
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
            &browser,
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

        let cmd_alt = |key| Some(Accelerator::new(Modifiers::META | Modifiers::ALT, key));
        let entry_items = vec![
            item(Action::BrowserOpen, "Open", None),
            item(Action::BrowserOpenApp, "Open in Default App", cmd(Code::KeyO)),
            item(Action::BrowserEdit, "Edit in $EDITOR", cmd(Code::KeyE)),
            item(Action::BrowserReveal, "Reveal in Finder", cmd(Code::KeyR)),
            item(Action::BrowserCd, "Go Here in the Shell", None),
            item(Action::BrowserInsert, "Insert Path at the Prompt", None),
            item(Action::BrowserCopyPath, "Copy Path", cmd(Code::KeyC)),
            item(Action::BrowserCopyRelative, "Copy Relative Path", cmd_alt(Code::KeyC)),
            item(Action::BrowserCopyName, "Copy Name", cmd_shift(Code::KeyC)),
            item(Action::BrowserRename, "Rename\u{2026}", None),
            item(Action::BrowserTrash, "Move to Trash", cmd(Code::Backspace)),
        ];
        let hidden = item(Action::BrowserHidden, "Show Hidden Files", cmd_shift(Code::Period));
        let entry = Menu::new();
        let [open, app, edit, reveal, cd, insert, path, relative, name, rename, trash] = &entry_items[..] else {
            unreachable!("eleven entry items")
        };
        let _ = entry.append_items(&[
            open,
            app,
            edit,
            reveal,
            &PredefinedMenuItem::separator(),
            cd,
            insert,
            path,
            relative,
            name,
            &PredefinedMenuItem::separator(),
            &item(Action::BrowserNewFolder, "New Folder\u{2026}", cmd_shift(Code::KeyN)),
            &item(Action::BrowserNewFile, "New File\u{2026}", cmd_alt(Code::KeyN)),
            rename,
            trash,
            &PredefinedMenuItem::separator(),
            &hidden,
            &item(Action::Browser, "Close File Browser", cmd(Code::KeyB)),
        ]);
        Self { menu, copy, flip, close, browser, entry, entry_items, hidden }
    }

    /// Shows the file browser's menu at the pointer.
    pub fn show_entry(&self, window: &Window, state: EntryState) {
        for (i, item) in self.entry_items.iter().enumerate() {
            // Editing needs a file; going there in the shell works on the
            // folder shown even with nothing chosen; the rest need an entry.
            item.set_enabled(match i {
                2 => state.has_entry && !state.folder,
                4 => true,
                _ => state.has_entry,
            });
        }
        self.hidden.set_text(if state.show_hidden { "Hide Hidden Files" } else { "Show Hidden Files" });
        show_at_pointer(&self.entry, window);
    }

    /// Shows the menu at the pointer. Returns once it is dismissed; the
    /// chosen item (if any) arrives as an event.
    pub fn show(&self, window: &Window, state: State) {
        self.browser.set_text(if state.browser { "Close File Browser" } else { "Show File Browser" });
        self.copy.set_enabled(state.has_selection);
        self.flip.set_text(if state.dark { "Switch to Light" } else { "Switch to Dark" });
        self.close.set_text(if state.panes > 1 {
            "Close Pane"
        } else if state.tabs > 1 {
            "Close Tab"
        } else {
            "Close Window"
        });
        show_at_pointer(&self.menu, window);
    }
}

/// Shows `menu` at the pointer. Returns once it is dismissed.
fn show_at_pointer(menu: &Menu, window: &Window) {
    let Ok(handle) = window.window_handle() else { return };
    #[cfg(target_os = "macos")]
    if let RawWindowHandle::AppKit(handle) = handle.as_raw() {
        // SAFETY: the view belongs to `window`, which outlives this call.
        unsafe {
            menu.show_context_menu_for_nsview(handle.ns_view.as_ptr(), None);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (menu, handle);
}
