//! Where the user's setup lives: plugins, grammars and `languages.toml`.

use std::path::PathBuf;

/// Arrow's user configuration directory: `%APPDATA%\arrow` on Windows,
/// `$XDG_CONFIG_HOME/arrow` or `~/.config/arrow` elsewhere.
pub fn user_config_dir() -> Option<PathBuf> {
    let config = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    config.map(|dir| dir.join("arrow"))
}

/// Where Arrow keeps things it can rebuild, like compiled plugins:
/// `%LOCALAPPDATA%\arrow\cache` on Windows (local, so it doesn't roam with
/// the user's profile), `$XDG_CACHE_HOME/arrow` or `~/.cache/arrow` elsewhere.
pub fn user_cache_dir() -> Option<PathBuf> {
    let cache = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("arrow").join("cache"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .map(|dir| dir.join("arrow"))
    };
    cache
}
