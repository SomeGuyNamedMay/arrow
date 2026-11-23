//! Turning key presses into the bytes a terminal program expects (xterm's
//! conventions, which almost every program understands).

use crate::components::keys::{EditorKey, KeyPress};

/// The bytes for a key press, or `None` for keys a terminal has no code for.
/// `application_cursor` is the mode full-screen programs switch on, in which
/// arrows send `ESC O A` instead of `ESC [ A`.
pub fn encode(press: KeyPress, application_cursor: bool) -> Option<Vec<u8>> {
    let mut bytes = match press.key {
        EditorKey::Char(c) if press.ctrl && !press.alt => return control(c).map(|b| vec![b]),
        EditorKey::Char(c) => c.to_string().into_bytes(),
        EditorKey::Enter => b"\r".to_vec(),
        EditorKey::Tab if press.shift => b"\x1b[Z".to_vec(),
        EditorKey::Tab => b"\t".to_vec(),
        EditorKey::Backspace => b"\x7f".to_vec(),
        EditorKey::Escape => b"\x1b".to_vec(),
        EditorKey::Delete => b"\x1b[3~".to_vec(),
        EditorKey::PageUp => b"\x1b[5~".to_vec(),
        EditorKey::PageDown => b"\x1b[6~".to_vec(),
        EditorKey::Up => cursor(b'A', application_cursor),
        EditorKey::Down => cursor(b'B', application_cursor),
        EditorKey::Right => cursor(b'C', application_cursor),
        EditorKey::Left => cursor(b'D', application_cursor),
        EditorKey::Home => cursor(b'H', application_cursor),
        EditorKey::End => cursor(b'F', application_cursor),
    };
    // Alt sends ESC first, as xterm's "meta sends escape" does. Ctrl+Alt is
    // AltGr on Windows, which types the character as it is.
    if press.alt && !press.ctrl {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

/// Ctrl with a character: `a`–`z` give 1–26, and a few symbols the rest.
fn control(c: char) -> Option<u8> {
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

fn cursor(code: u8, application: bool) -> Vec<u8> {
    vec![0x1b, if application { b'O' } else { b'[' }, code]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: EditorKey, ctrl: bool, alt: bool) -> KeyPress {
        KeyPress {
            key,
            alt,
            ctrl,
            shift: false,
        }
    }

    fn bytes(key: EditorKey) -> Vec<u8> {
        encode(KeyPress::plain(key), false).unwrap()
    }

    #[test]
    fn keys_become_xterm_bytes() {
        assert_eq!(bytes(EditorKey::Char('é')), "é".as_bytes());
        assert_eq!(bytes(EditorKey::Enter), b"\r");
        assert_eq!(bytes(EditorKey::Backspace), b"\x7f");
        assert_eq!(bytes(EditorKey::Up), b"\x1b[A");
        assert_eq!(bytes(EditorKey::Delete), b"\x1b[3~");
        assert_eq!(bytes(EditorKey::PageDown), b"\x1b[6~");
        assert_eq!(encode(KeyPress::plain(EditorKey::Up), true).unwrap(), b"\x1bOA");
        assert_eq!(encode(press(EditorKey::Char('c'), true, false), false).unwrap(), [3]);
        assert_eq!(encode(press(EditorKey::Char('D'), true, false), false).unwrap(), [4]);
        assert_eq!(encode(press(EditorKey::Char('b'), false, true), false).unwrap(), b"\x1bb");
        // AltGr (Ctrl+Alt) types the character itself.
        assert_eq!(encode(press(EditorKey::Char('@'), true, true), false).unwrap(), b"@");
        assert_eq!(encode(press(EditorKey::Char('%'), true, false), false), None);
    }
}
