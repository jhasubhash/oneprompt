//! Keyboard events to the byte sequences a shell expects (xterm conventions).

use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

/// Returns the bytes to send for a key press, or `None` if the key is not
/// meant for the shell (e.g. Cmd shortcuts, which the app handles itself).
///
/// With `option_as_meta`, Alt/Option sends ESC + key (Meta) instead of the
/// character the OS would compose; callers enable it for the left Option key
/// only, so the right one still types characters like é or €.
pub fn key_to_bytes(
    event: &KeyEvent,
    mods: ModifiersState,
    option_as_meta: bool,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    if mods.super_key() {
        return None;
    }

    // xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
    let modifier = 1
        + mods.shift_key() as u8
        + 2 * mods.alt_key() as u8
        + 4 * mods.control_key() as u8;

    let cursor = |letter: char| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{letter}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[{code};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    };

    if let Key::Named(named) = &event.logical_key {
        let bytes = match named {
            NamedKey::Enter => b"\r".to_vec(),
            NamedKey::Backspace if mods.alt_key() => b"\x1b\x7f".to_vec(),
            NamedKey::Backspace => b"\x7f".to_vec(),
            NamedKey::Tab if mods.shift_key() => b"\x1b[Z".to_vec(),
            NamedKey::Tab => b"\t".to_vec(),
            NamedKey::Escape => b"\x1b".to_vec(),
            NamedKey::Space if mods.control_key() => vec![0],
            NamedKey::Space => b" ".to_vec(),
            NamedKey::ArrowUp => cursor('A'),
            NamedKey::ArrowDown => cursor('B'),
            // Option+arrows move by word, as in other macOS terminals.
            NamedKey::ArrowRight if mods == ModifiersState::ALT => b"\x1bf".to_vec(),
            NamedKey::ArrowLeft if mods == ModifiersState::ALT => b"\x1bb".to_vec(),
            NamedKey::ArrowRight => cursor('C'),
            NamedKey::ArrowLeft => cursor('D'),
            NamedKey::Home => cursor('H'),
            NamedKey::End => cursor('F'),
            NamedKey::Insert => tilde(2),
            NamedKey::Delete => tilde(3),
            NamedKey::PageUp => tilde(5),
            NamedKey::PageDown => tilde(6),
            NamedKey::F1 => b"\x1bOP".to_vec(),
            NamedKey::F2 => b"\x1bOQ".to_vec(),
            NamedKey::F3 => b"\x1bOR".to_vec(),
            NamedKey::F4 => b"\x1bOS".to_vec(),
            NamedKey::F5 => tilde(15),
            NamedKey::F6 => tilde(17),
            NamedKey::F7 => tilde(18),
            NamedKey::F8 => tilde(19),
            NamedKey::F9 => tilde(20),
            NamedKey::F10 => tilde(21),
            NamedKey::F11 => tilde(23),
            NamedKey::F12 => tilde(24),
            _ => return None,
        };
        return Some(bytes);
    }

    if option_as_meta && mods.alt_key() {
        let key = event.key_without_modifiers();
        if let Key::Character(s) = &key {
            let mut chars = s.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                let c = if mods.shift_key() { c.to_ascii_uppercase() } else { c };
                let mut bytes = vec![0x1b];
                match (mods.control_key(), control_byte(c)) {
                    (true, Some(b)) => bytes.push(b),
                    _ => bytes.extend(c.to_string().as_bytes()),
                }
                return Some(bytes);
            }
        }
    }

    if mods.control_key()
        && let Key::Character(s) = &event.logical_key
    {
        let mut chars = s.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && let Some(byte) = control_byte(c)
        {
            return Some(vec![byte]);
        }
    }

    event.text.as_ref().map(|t| t.as_bytes().to_vec())
}

/// The C0 control byte for Ctrl+`c`, if there is one.
fn control_byte(c: char) -> Option<u8> {
    Some(match c.to_ascii_lowercase() {
        c @ 'a'..='z' => c as u8 & 0x1f,
        '@' | '2' | ' ' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}
