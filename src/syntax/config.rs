//! Which grammar to use for which files, and the colours to draw captures in,
//! from `languages.toml` in the user config directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use iced::Color;
use serde::Deserialize;

/// Colours for common capture names, used unless the theme says otherwise.
pub(super) const DEFAULT_THEME: &[(&str, Color)] = &[
    ("attribute", Color::from_rgb8(229, 192, 123)),
    ("comment", Color::from_rgb8(110, 115, 135)),
    ("constant", Color::from_rgb8(209, 154, 102)),
    ("constant.builtin", Color::from_rgb8(209, 154, 102)),
    ("constructor", Color::from_rgb8(229, 192, 123)),
    ("escape", Color::from_rgb8(86, 182, 194)),
    ("function", Color::from_rgb8(97, 175, 239)),
    ("function.macro", Color::from_rgb8(86, 182, 194)),
    ("keyword", Color::from_rgb8(198, 120, 221)),
    ("label", Color::from_rgb8(224, 108, 117)),
    ("number", Color::from_rgb8(209, 154, 102)),
    ("operator", Color::from_rgb8(171, 178, 191)),
    ("property", Color::from_rgb8(224, 108, 117)),
    ("punctuation", Color::from_rgb8(150, 155, 170)),
    ("string", Color::from_rgb8(152, 195, 121)),
    ("type", Color::from_rgb8(229, 192, 123)),
    ("type.builtin", Color::from_rgb8(229, 192, 123)),
    ("variable.builtin", Color::from_rgb8(224, 108, 117)),
    ("variable.parameter", Color::from_rgb8(220, 220, 225)),
];

#[derive(Debug, Clone, Deserialize)]
pub(super) struct LanguageConfig {
    pub(super) name: String,
    #[serde(default)]
    extensions: Vec<String>,
    /// File stem in `grammars/`. Defaults to the name.
    pub(super) grammar: Option<String>,
    /// Path of the highlights query, relative to the config directory.
    pub(super) highlights: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    language: Vec<LanguageConfig>,
    #[serde(default)]
    theme: HashMap<String, String>,
}

/// The languages the user configured, and the colours to draw them with.
pub struct Languages {
    /// Where grammars and queries are found.
    pub(super) dir: PathBuf,
    languages: Vec<LanguageConfig>,
    theme: HashMap<String, Color>,
}

impl Languages {
    /// No languages: nothing is highlighted.
    pub fn none() -> Self {
        Self {
            dir: PathBuf::new(),
            languages: Vec::new(),
            theme: default_theme(),
        }
    }

    /// Read `languages.toml` from `dir`. A missing file means no languages.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join("languages.toml");
        let config: ConfigFile = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ConfigFile::default(),
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let mut theme = default_theme();
        for (capture, hex) in config.theme {
            let color = parse_color(&hex)
                .ok_or_else(|| format!("{}: `{hex}` is not a #rrggbb colour", path.display()))?;
            theme.insert(capture, color);
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            languages: config.language,
            theme,
        })
    }

    /// The language for a buffer, by its name's extension.
    pub(super) fn for_buffer(&self, name: &str) -> Option<&LanguageConfig> {
        let extension = Path::new(name).extension()?.to_str()?;
        self.languages.iter().find(|language| {
            language
                .extensions
                .iter()
                .any(|e| e.eq_ignore_ascii_case(extension))
        })
    }

    /// The colour for a capture name, falling back from "a.b.c" to "a.b" to "a".
    pub(super) fn color(&self, capture: &str) -> Option<Color> {
        let mut name = capture;
        loop {
            if let Some(color) = self.theme.get(name) {
                return Some(*color);
            }
            name = &name[..name.rfind('.')?];
        }
    }
}

fn default_theme() -> HashMap<String, Color> {
    DEFAULT_THEME
        .iter()
        .map(|(name, color)| (name.to_string(), *color))
        .collect()
}

pub(super) fn parse_color(hex: &str) -> Option<Color> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some(Color::from_rgb8(channel(0)?, channel(2)?, channel(4)?))
}
