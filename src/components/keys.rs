//! Key presses shared by the components, conversion from iced keyboard events, and
//! parsing of key sequences written in Kakoune's notation (`"gh"`, `"<a-s>"`,
//! `"<space>d"`, `"<esc>"`; `<lt>` and `<gt>` for literal `<` and `>`).

use std::fmt;

use iced::keyboard::{self, Key, key::Named};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditorKey {
    Char(char),
    Escape,
    Enter,
    Backspace,
    Delete,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPress {
    pub key: EditorKey,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

impl KeyPress {
    pub fn plain(key: EditorKey) -> Self {
        Self {
            key,
            alt: false,
            ctrl: false,
            shift: false,
        }
    }

    /// The character this press would type, if any. Ctrl+Alt counts as typing
    /// because it is AltGr on Windows, which types characters on many layouts.
    pub fn typed_char(&self) -> Option<char> {
        match self.key {
            EditorKey::Char(c) if !self.ctrl || self.alt => Some(c),
            _ => None,
        }
    }
}

/// A key as used in bindings. For characters, shift is already in the character
/// itself (`W` rather than shift+`w`), so it is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub key: EditorKey,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

impl From<KeyPress> for KeyChord {
    fn from(press: KeyPress) -> Self {
        Self {
            key: press.key,
            alt: press.alt,
            ctrl: press.ctrl,
            shift: press.shift && !matches!(press.key, EditorKey::Char(_)),
        }
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let base = match self.key {
            EditorKey::Char('<') => "lt".to_string(),
            EditorKey::Char('>') => "gt".to_string(),
            EditorKey::Char(' ') => "space".to_string(),
            EditorKey::Char(c) => c.to_string(),
            EditorKey::Escape => "esc".to_string(),
            EditorKey::Enter => "ret".to_string(),
            EditorKey::Backspace => "backspace".to_string(),
            EditorKey::Delete => "del".to_string(),
            EditorKey::Tab => "tab".to_string(),
            EditorKey::Left => "left".to_string(),
            EditorKey::Right => "right".to_string(),
            EditorKey::Up => "up".to_string(),
            EditorKey::Down => "down".to_string(),
            EditorKey::Home => "home".to_string(),
            EditorKey::End => "end".to_string(),
        };
        let plain = matches!(self.key, EditorKey::Char(c) if !matches!(c, '<' | '>' | ' '))
            && !self.alt
            && !self.ctrl
            && !self.shift;
        if plain {
            return write!(f, "{base}");
        }
        let ctrl = if self.ctrl { "c-" } else { "" };
        let alt = if self.alt { "a-" } else { "" };
        let shift = if self.shift { "s-" } else { "" };
        write!(f, "<{ctrl}{alt}{shift}{base}>")
    }
}

/// Parse a key sequence such as `"gh"` or `"<a-;>x"`.
pub fn parse_keys(keys: &str) -> Result<Vec<KeyChord>, String> {
    let mut chords = Vec::new();
    let mut chars = keys.chars();
    while let Some(c) = chars.next() {
        if c != '<' {
            chords.push(KeyChord::from(KeyPress::plain(EditorKey::Char(c))));
            continue;
        }
        let mut name = String::new();
        let mut closed = false;
        for c in chars.by_ref() {
            if c == '>' {
                closed = true;
                break;
            }
            name.push(c);
        }
        if !closed {
            return Err(format!("unclosed `<` in `{keys}`"));
        }
        chords.push(parse_named(&name)?);
    }
    if chords.is_empty() {
        return Err("empty key sequence".into());
    }
    Ok(chords)
}

fn parse_named(name: &str) -> Result<KeyChord, String> {
    let (mut alt, mut ctrl, mut shift) = (false, false, false);
    let mut rest = name;
    while rest.chars().count() > 2 {
        if let Some(r) = rest.strip_prefix("a-") {
            alt = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("c-") {
            ctrl = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("s-") {
            shift = true;
            rest = r;
        } else {
            break;
        }
    }
    let key = match rest {
        "esc" => EditorKey::Escape,
        "ret" | "enter" => EditorKey::Enter,
        "tab" => EditorKey::Tab,
        "backspace" => EditorKey::Backspace,
        "del" => EditorKey::Delete,
        "left" => EditorKey::Left,
        "right" => EditorKey::Right,
        "up" => EditorKey::Up,
        "down" => EditorKey::Down,
        "home" => EditorKey::Home,
        "end" => EditorKey::End,
        "lt" => EditorKey::Char('<'),
        "gt" => EditorKey::Char('>'),
        "space" => EditorKey::Char(' '),
        "minus" => EditorKey::Char('-'),
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => EditorKey::Char(c),
                _ => return Err(format!("unknown key `<{name}>`")),
            }
        }
    };
    // `<s-x>` means `X`; shift only stays on non-character keys.
    let key = match key {
        EditorKey::Char(c) if shift => {
            shift = false;
            EditorKey::Char(c.to_uppercase().next().unwrap_or(c))
        }
        key => key,
    };
    Ok(KeyChord {
        key,
        alt,
        ctrl,
        shift,
    })
}

/// Format a key sequence back into Kakoune notation.
pub fn format_keys(chords: &[KeyChord]) -> String {
    chords.iter().map(KeyChord::to_string).collect()
}

/// Turn an iced keyboard event into a key press, ignoring releases and keys
/// the components don't use.
pub fn from_event(event: keyboard::Event) -> Option<KeyPress> {
    let keyboard::Event::KeyPressed {
        modified_key,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    // `modified_key` has shift applied, so shift+w arrives as `W`.
    let key = match modified_key.as_ref() {
        Key::Character(s) => EditorKey::Char(s.chars().next()?),
        Key::Named(Named::Space) => EditorKey::Char(' '),
        Key::Named(Named::Escape) => EditorKey::Escape,
        Key::Named(Named::Enter) => EditorKey::Enter,
        Key::Named(Named::Backspace) => EditorKey::Backspace,
        Key::Named(Named::Delete) => EditorKey::Delete,
        Key::Named(Named::Tab) => EditorKey::Tab,
        Key::Named(Named::ArrowLeft) => EditorKey::Left,
        Key::Named(Named::ArrowRight) => EditorKey::Right,
        Key::Named(Named::ArrowUp) => EditorKey::Up,
        Key::Named(Named::ArrowDown) => EditorKey::Down,
        Key::Named(Named::Home) => EditorKey::Home,
        Key::Named(Named::End) => EditorKey::End,
        _ => return None,
    };
    Some(KeyPress {
        key,
        alt: modifiers.alt(),
        ctrl: modifiers.control(),
        shift: modifiers.shift(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_key_sequences() {
        for keys in ["gh", "<a-;>", "<s-left>", "<esc>", "<lt>", "<c-a-x>", "<space>d"] {
            assert_eq!(format_keys(&parse_keys(keys).unwrap()), keys);
        }
        assert_eq!(parse_keys("<s-w>").unwrap(), parse_keys("W").unwrap());
        assert!(parse_keys("<bogus>").is_err());
        assert!(parse_keys("<a-").is_err());
        assert!(parse_keys("").is_err());
    }

    #[test]
    fn shift_is_dropped_for_characters() {
        let press = KeyPress {
            shift: true,
            ..KeyPress::plain(EditorKey::Char('W'))
        };
        assert_eq!(KeyChord::from(press), parse_keys("W").unwrap()[0]);
    }
}
