//! What plugins have registered: modes, commands, key bindings, prefix labels
//! and event subscriptions. Plugins fill it in through the `registry` interface;
//! the session reads it to turn keys into commands.

use std::collections::HashMap;

use super::EventKind;
use crate::components::keys::{KeyChord, format_keys, parse_keys};

pub struct PluginCommand {
    /// Index of the plugin that implements it.
    pub plugin: usize,
    pub description: String,
    /// Wait for a typed character and pass it to the command.
    pub takes_char: bool,
}

struct Binding {
    mode: String,
    keys: Vec<KeyChord>,
    command: String,
}

pub enum Lookup {
    Command(String),
    /// The keys so far start a longer binding.
    Prefix,
    Unbound,
}

/// Modes, commands, key bindings and event subscriptions added by plugins.
#[derive(Default)]
pub struct Registry {
    /// Mode name to its fallback command for typed characters.
    modes: HashMap<String, Option<String>>,
    commands: HashMap<String, PluginCommand>,
    bindings: Vec<Binding>,
    /// Labels for prefix keys, keyed by mode and keys.
    prefix_labels: HashMap<(String, Vec<KeyChord>), String>,
    subscriptions: Vec<(usize, EventKind)>,
    /// Names of loaded plugins, by index.
    plugin_names: Vec<String>,
}

impl Registry {
    // ----- Filled in by plugins -----

    /// Record a loaded plugin's name; its index is its position.
    pub fn add_plugin(&mut self, name: &str) {
        self.plugin_names.push(name.to_string());
    }

    /// Create a mode, or change its fallback.
    pub fn define_mode(&mut self, name: String, fallback: Option<String>) {
        self.modes.insert(name, fallback);
    }

    /// Add a command, replacing one with the same name.
    pub fn register_command(&mut self, name: String, command: PluginCommand) {
        self.commands.insert(name, command);
    }

    pub fn name_prefix(&mut self, mode: String, keys: &str, label: String) -> Result<(), String> {
        let keys = parse_keys(keys)?;
        self.prefix_labels.insert((mode, keys), label);
        Ok(())
    }

    pub fn subscribe(&mut self, plugin: usize, kind: EventKind) {
        if !self.subscriptions.contains(&(plugin, kind)) {
            self.subscriptions.push((plugin, kind));
        }
    }

    // ----- Read by the session and by plugins -----

    pub fn command(&self, name: &str) -> Option<&PluginCommand> {
        self.commands.get(name)
    }

    /// Every plugin command, by name, in no particular order.
    pub fn commands(&self) -> impl Iterator<Item = (&str, &PluginCommand)> {
        self.commands.iter().map(|(name, command)| (name.as_str(), command))
    }

    /// The name of the plugin at `index`, in load order.
    pub fn plugin_name(&self, index: usize) -> &str {
        self.plugin_names.get(index).map_or("", String::as_str)
    }

    /// Every key binding as (mode, keys in Kakoune notation, command), in the
    /// order they were bound.
    pub fn bindings(&self) -> impl Iterator<Item = (&str, String, &str)> {
        self.bindings
            .iter()
            .map(|b| (b.mode.as_str(), format_keys(&b.keys), b.command.as_str()))
    }

    pub fn has_mode(&self, mode: &str) -> bool {
        self.modes.contains_key(mode)
    }

    /// The command that receives typed characters in `mode`, if any.
    pub fn fallback(&self, mode: &str) -> Option<&str> {
        self.modes.get(mode)?.as_deref()
    }

    pub fn bind_key(&mut self, mode: &str, keys: &str, command: &str) -> Result<(), String> {
        let keys = parse_keys(keys)?;
        self.bindings.retain(|b| !(b.mode == mode && b.keys == keys));
        self.bindings.push(Binding {
            mode: mode.to_string(),
            keys,
            command: command.to_string(),
        });
        Ok(())
    }

    pub fn lookup(&self, mode: &str, keys: &[KeyChord]) -> Lookup {
        let mut prefix = false;
        for binding in self.bindings.iter().filter(|b| b.mode == mode) {
            if binding.keys == keys {
                return Lookup::Command(binding.command.clone());
            }
            prefix |= binding.keys.starts_with(keys);
        }
        if prefix { Lookup::Prefix } else { Lookup::Unbound }
    }

    /// What can follow `keys` in `mode`, for the status bar, like Kakoune's info
    /// box: "goto: h goto-line-start · l goto-line-end · …".
    pub fn prefix_hint(&self, mode: &str, keys: &[KeyChord]) -> String {
        let mut items: Vec<String> = Vec::new();
        for binding in self.bindings.iter().filter(|b| b.mode == mode) {
            if binding.keys.len() <= keys.len() || !binding.keys.starts_with(keys) {
                continue;
            }
            let next = binding.keys[keys.len()];
            let item = if binding.keys.len() == keys.len() + 1 {
                format!("{next} {}", binding.command)
            } else {
                format!("{next}…")
            };
            if !items.contains(&item) {
                items.push(item);
            }
        }
        let label = self
            .prefix_labels
            .get(&(mode.to_string(), keys.to_vec()))
            .cloned()
            .unwrap_or_else(|| format_keys(keys));
        format!("{label}: {}", items.join(" · "))
    }

    /// The normal-mode key binding for a command, if it has one.
    pub fn binding_for(&self, command: &str) -> Option<String> {
        self.bindings
            .iter()
            .find(|b| b.mode == "normal" && b.command == command)
            .map(|b| format_keys(&b.keys))
    }

    pub fn has_subscribers(&self, kind: EventKind) -> bool {
        self.subscriptions.iter().any(|(_, k)| *k == kind)
    }

    /// Plugins subscribed to `kind`, in load order.
    pub fn subscribers(&self, kind: EventKind) -> Vec<usize> {
        self.subscriptions
            .iter()
            .filter(|(_, k)| *k == kind)
            .map(|(plugin, _)| *plugin)
            .collect()
    }
}
