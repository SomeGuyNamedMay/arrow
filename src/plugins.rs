//! WebAssembly component plugins, run with wasmtime.
//!
//! Plugins target the `plugin` world in `wit/arrow.wit`. They define modes, add
//! commands, bind keys and subscribe to editor events. The editor itself only
//! stores text and dots; all movement and editing behaviour, including keeping
//! dots in place across edits and undo, comes from the bundled `core` plugin.
//!
//! Each plugin gets its own store and instance, so plugins can't see each other's
//! memory. During a plugin call the app's [`Core`] (the editor plus the registry)
//! is moved into that plugin's store so the host functions can use it, then moved
//! back.
//!
//! Calls are async (wasmtime's `call_async`), so the app runs them off the UI
//! thread. Every epoch tick a running plugin yields to the executor, and once its
//! time limit is spent it is interrupted; a plugin that traps or times out is
//! disabled.

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, UpdateDeadline};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxView, WasiView};

use crate::components::keys::{KeyChord, format_keys, parse_keys};
use crate::components::menu::{MenuItem, MenuSpec};
use crate::components::tree_menu::{TreeNode, TreeSpec};
use crate::components::text_editor::{Selection, TextEditor};

wasmtime::component::bindgen!({
    world: "plugin",
    path: "wit",
    exports: { default: async },
});

use self::arrow::plugin::{editor as host_editor, registry as host_registry, types, ui as host_ui};
// `Event` is already defined at this level by bindgen, because the world `use`s it.
pub use self::arrow::plugin::types::EventKind;

/// Plugins built from `plugins/` by `build.rs` and embedded in the executable.
/// Core loads first so the others can bind its commands.
const BUNDLED: &[(&str, &[u8], Access)] = &[
    (
        "core",
        include_bytes!(concat!(env!("OUT_DIR"), "/arrow_core.wasm")),
        Access::None,
    ),
    (
        "text-tools",
        include_bytes!(concat!(env!("OUT_DIR"), "/text_tools.wasm")),
        Access::None,
    ),
    (
        "files",
        include_bytes!(concat!(env!("OUT_DIR"), "/files.wasm")),
        Access::ReadWorkingDir,
    ),
];

/// What a plugin may reach outside its sandbox.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    /// Nothing but stdout and stderr (for `println!` debugging).
    None,
    /// Also read, but not write, the directory Arrow was started in. Inside the
    /// plugin it is the current directory, ".".
    ReadWorkingDir,
}

/// How often the epoch counter advances.
const TICK: Duration = Duration::from_millis(10);
/// Time limit for a command or event hook: about half a second of ticks.
const CALL_TICKS: u64 = 50;
/// Time limit for a plugin's `init`: about five seconds.
const INIT_TICKS: u64 = 500;

/// One engine for the whole process, with a background thread driving epochs.
fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut config = Config::new();
        config.epoch_interruption(true);
        let engine = Engine::new(&config).expect("valid wasmtime configuration");
        let ticker = engine.clone();
        std::thread::Builder::new()
            .name("plugin-epoch".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(TICK);
                    ticker.increment_epoch();
                }
            })
            .expect("failed to start the plugin epoch thread");
        engine
    })
}

/// The bundled plugins, compiled once per process (compiling is the slow part).
fn bundled_components() -> &'static [(&'static str, Result<Component, String>)] {
    static COMPILED: OnceLock<Vec<(&'static str, Result<Component, String>)>> = OnceLock::new();
    COMPILED.get_or_init(|| {
        BUNDLED
            .iter()
            .map(|(name, bytes, _)| {
                let component = Component::new(engine(), bytes)
                    .map_err(|e| format!("plugin `{name}` failed to compile: {e:#}"));
                (*name, component)
            })
            .collect()
    })
}

/// Where user plugins live: `%APPDATA%\arrow\plugins` on Windows,
/// `$XDG_CONFIG_HOME/arrow/plugins` or `~/.config/arrow/plugins` elsewhere.
pub fn user_plugin_dir() -> Option<PathBuf> {
    let config = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    config.map(|dir| dir.join("arrow").join("plugins"))
}

// ----- Registry -----

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
    pub fn command(&self, name: &str) -> Option<&PluginCommand> {
        self.commands.get(name)
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

/// What plugins can see and change: the editor, the registry, and the queues of
/// work they've asked for.
#[derive(Default)]
pub struct Core {
    pub editor: TextEditor,
    pub registry: Registry,
    /// Events waiting to be delivered.
    pub events: VecDeque<Event>,
    /// Commands (with arguments) plugins asked to run once their call returns.
    pub queued_commands: VecDeque<(String, Option<String>)>,
    /// A menu a plugin asked to show once its call returns.
    pub menu_request: Option<MenuSpec>,
    /// A tree menu a plugin asked to show once its call returns.
    pub tree_request: Option<TreeSpec>,
    /// Nodes plugins added to the open tree menu, as (parent, nodes).
    pub tree_additions: Vec<(Option<String>, Vec<TreeNode>)>,
    /// Commands provided by the app itself rather than a plugin (like `quit`), as
    /// (name, description).
    pub app_commands: Vec<(&'static str, &'static str)>,
}

impl Core {
    pub fn is_command(&self, name: &str) -> bool {
        self.registry.command(name).is_some() || self.is_app_command(name)
    }

    pub fn is_app_command(&self, name: &str) -> bool {
        self.app_commands.iter().any(|(n, _)| *n == name)
    }

    /// Every command plugins and the app provide, sorted by name.
    fn list_commands(&self) -> Vec<types::CommandInfo> {
        let registry = &self.registry;
        let plugin_commands = registry.commands.iter().map(|(name, command)| types::CommandInfo {
            name: name.clone(),
            description: command.description.clone(),
            plugin: registry
                .plugin_names
                .get(command.plugin)
                .cloned()
                .unwrap_or_default(),
            binding: registry.binding_for(name),
            takes_char: command.takes_char,
        });
        let app_commands = self.app_commands.iter().map(|(name, description)| types::CommandInfo {
            name: name.to_string(),
            description: description.to_string(),
            plugin: "app".to_string(),
            binding: registry.binding_for(name),
            takes_char: false,
        });
        let mut commands: Vec<_> = plugin_commands.chain(app_commands).collect();
        commands.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        commands
    }
}

// ----- Host functions -----

/// A plugin store's data. `core` is only present during a call.
pub struct HostState {
    core: Option<Core>,
    plugin: usize,
    /// Epoch ticks the current call may still run for.
    ticks_left: u64,
    wasi: WasiCtx,
    table: ResourceTable,
}

impl HostState {
    fn core(&mut self) -> &mut Core {
        self.core
            .as_mut()
            .expect("plugins only run while the host has lent them the core")
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl From<Selection> for types::Selection {
    fn from(selection: Selection) -> Self {
        types::Selection {
            anchor: selection.anchor as u32,
            cursor: selection.cursor as u32,
        }
    }
}

impl From<types::Selection> for Selection {
    fn from(selection: types::Selection) -> Self {
        Selection {
            anchor: selection.anchor as usize,
            cursor: selection.cursor as usize,
        }
    }
}

impl types::Host for HostState {}

impl host_editor::Host for HostState {
    fn len(&mut self) -> u32 {
        self.core().editor.editor().len() as u32
    }

    fn char_at(&mut self, index: u32) -> Option<char> {
        self.core().editor.editor().char_at(index as usize)
    }

    fn slice(&mut self, start: u32, end: u32) -> String {
        self.core().editor.editor().slice(start as usize, end as usize)
    }

    fn get_selection(&mut self) -> types::Selection {
        self.core().editor.editor().selection().into()
    }

    fn set_selection(&mut self, selection: types::Selection) {
        self.core().editor.editor_mut().set_selection(selection.into());
    }

    fn get_dots(&mut self) -> Vec<types::Selection> {
        let dots = self.core().editor.editor().dots();
        dots.iter().map(|&dot| dot.into()).collect()
    }

    fn set_dots(&mut self, dots: Vec<types::Selection>) {
        let dots: Vec<Selection> = dots.into_iter().map(Selection::from).collect();
        self.core().editor.editor_mut().set_dots(&dots);
    }

    fn get_dot(&mut self, index: u32) -> Option<types::Selection> {
        self.core().editor.editor().dot(index as usize).map(Into::into)
    }

    fn set_dot(&mut self, index: u32, selection: types::Selection) {
        self.core()
            .editor
            .editor_mut()
            .set_dot(index as usize, selection.into());
    }

    fn replace(&mut self, start: u32, end: u32, text: String) {
        self.core()
            .editor
            .editor_mut()
            .replace(start as usize, end as usize, &text);
    }

    fn get_mode(&mut self) -> String {
        self.core().editor.editor().mode().to_string()
    }

    fn get_buffer_name(&mut self) -> String {
        self.core().editor.editor().name().to_string()
    }

    fn set_buffer_name(&mut self, name: String) {
        self.core().editor.editor_mut().set_name(name);
    }

    fn set_mode(&mut self, mode: String) -> Result<(), String> {
        let core = self.core();
        if !core.registry.has_mode(&mode) {
            return Err(format!("no such mode: {mode}"));
        }
        let inserting = core.registry.fallback(&mode).is_some();
        core.editor.editor_mut().set_mode(&mode, inserting);
        Ok(())
    }

    fn message(&mut self, text: String) {
        self.core().editor.editor_mut().set_message(text);
    }

    fn run_command(&mut self, name: String, arg: Option<String>) -> bool {
        // Always queued: running another plugin's command now could call back into
        // the plugin that is still running, which components don't allow.
        let core = self.core();
        let exists = core.is_command(&name);
        if exists {
            core.queued_commands.push_back((name, arg));
        }
        exists
    }
}

impl From<types::TreeNode> for TreeNode {
    fn from(node: types::TreeNode) -> Self {
        TreeNode {
            label: node.label,
            value: node.value,
            annotation: node.annotation,
            detail: node.detail,
            parent: node.parent,
            branch: node.branch,
        }
    }
}

impl host_ui::Host for HostState {
    fn open_tree_menu(&mut self, menu: types::TreeMenu) {
        self.core().tree_request = Some(TreeSpec {
            title: menu.title,
            nodes: menu.nodes.into_iter().map(TreeNode::from).collect(),
            on_select: menu.on_select,
            on_expand: menu.on_expand,
        });
    }

    fn add_tree_nodes(&mut self, parent: Option<String>, nodes: Vec<types::TreeNode>) {
        let nodes = nodes.into_iter().map(TreeNode::from).collect();
        self.core().tree_additions.push((parent, nodes));
    }

    fn open_menu(&mut self, menu: types::Menu) {
        self.core().menu_request = Some(MenuSpec {
            prompt: menu.prompt,
            input: menu.input,
            items: menu
                .items
                .into_iter()
                .map(|item| MenuItem {
                    label: item.label,
                    value: item.value,
                    annotation: item.annotation,
                    detail: item.detail,
                })
                .collect(),
            on_select: menu.on_select,
        });
    }
}

impl host_registry::Host for HostState {
    fn define_mode(&mut self, name: String, fallback: Option<String>) {
        self.core().registry.modes.insert(name, fallback);
    }

    fn register_command(&mut self, name: String, description: String, takes_char: bool) {
        let plugin = self.plugin;
        self.core().registry.commands.insert(
            name,
            PluginCommand {
                plugin,
                description,
                takes_char,
            },
        );
    }

    fn bind_key(&mut self, mode: String, keys: String, command: String) -> Result<(), String> {
        let registry = &mut self.core().registry;
        if !registry.has_mode(&mode) {
            return Err(format!("no such mode: {mode}"));
        }
        registry.bind_key(&mode, &keys, &command)
    }

    fn name_prefix(&mut self, mode: String, keys: String, label: String) -> Result<(), String> {
        let keys = parse_keys(&keys)?;
        self.core().registry.prefix_labels.insert((mode, keys), label);
        Ok(())
    }

    fn subscribe(&mut self, event: EventKind) {
        let plugin = self.plugin;
        let subscriptions = &mut self.core().registry.subscriptions;
        if !subscriptions.contains(&(plugin, event)) {
            subscriptions.push((plugin, event));
        }
    }

    fn list_commands(&mut self) -> Vec<types::CommandInfo> {
        self.core().list_commands()
    }

    fn list_bindings(&mut self) -> Vec<types::BindingInfo> {
        self.core()
            .registry
            .bindings
            .iter()
            .map(|binding| types::BindingInfo {
                mode: binding.mode.clone(),
                keys: format_keys(&binding.keys),
                command: binding.command.clone(),
            })
            .collect()
    }
}

// ----- Loading and calling plugins -----

struct LoadedPlugin {
    name: String,
    store: Store<HostState>,
    bindings: Plugin,
    /// Set after a trap or timeout. The instance can't safely be used again.
    failed: bool,
}

pub struct PluginHost {
    linker: Linker<HostState>,
    plugins: Vec<LoadedPlugin>,
}

impl PluginHost {
    pub fn new() -> Self {
        let mut linker = Linker::new(engine());
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("WASI links");
        Plugin::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .expect("plugin interfaces link");
        Self {
            linker,
            plugins: Vec::new(),
        }
    }

    pub async fn load_bundled(&mut self, core: &mut Core) -> Vec<String> {
        let mut errors = Vec::new();
        let access = BUNDLED.iter().map(|(_, _, access)| *access);
        for ((name, component), access) in bundled_components().iter().zip(access) {
            let result = match component {
                Ok(component) => self.load(name, component, core, access).await,
                Err(error) => Err(error.clone()),
            };
            if let Err(error) = result {
                errors.push(error);
            }
        }
        errors
    }

    /// Load every `*.wasm` in the user plugin directory, in name order.
    pub async fn load_user_plugins(&mut self, core: &mut Core) -> Vec<String> {
        let Some(dir) = user_plugin_dir() else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut paths: Vec<_> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "wasm"))
            .collect();
        paths.sort();
        let mut errors = Vec::new();
        for path in &paths {
            if let Err(error) = self.load_file(path, core).await {
                errors.push(error);
            }
        }
        errors
    }

    async fn load_file(&mut self, path: &Path, core: &mut Core) -> Result<(), String> {
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let component =
            Component::from_file(engine(), path).map_err(|e| format!("plugin `{name}`: {e:#}"))?;
        // User plugins get no access outside their sandbox.
        self.load(&name, &component, core, Access::None).await
    }

    /// Instantiate a plugin and run its `init`.
    async fn load(
        &mut self,
        name: &str,
        component: &Component,
        core: &mut Core,
        access: Access,
    ) -> Result<(), String> {
        // No network or environment; stdout and stderr so `println!` debugging
        // works; and a read-only view of the working directory if granted.
        let mut wasi = WasiCtx::builder();
        wasi.inherit_stdout().inherit_stderr();
        if access == Access::ReadWorkingDir {
            wasi.preopened_dir(".", ".", FsPerms::ReadOnly)
                .map_err(|e| format!("plugin `{name}`: can't open the working directory: {e:#}"))?;
        }
        let wasi = wasi.build();
        let mut store = Store::new(
            engine(),
            HostState {
                core: None,
                plugin: self.plugins.len(),
                ticks_left: INIT_TICKS,
                wasi,
                table: ResourceTable::new(),
            },
        );
        // Yield to the executor on every tick, until the call's ticks run out.
        store.epoch_deadline_callback(|mut state| {
            let ticks_left = &mut state.data_mut().ticks_left;
            if *ticks_left == 0 {
                return Ok(UpdateDeadline::Interrupt);
            }
            *ticks_left -= 1;
            Ok(UpdateDeadline::Yield(1))
        });
        store.set_epoch_deadline(1);
        let bindings = Plugin::instantiate_async(&mut store, component, &self.linker)
            .await
            .map_err(|e| format!("plugin `{name}` failed to load: {e:#}"))?;
        self.plugins.push(LoadedPlugin {
            name: name.to_string(),
            store,
            bindings,
            failed: false,
        });
        core.registry.plugin_names.push(name.to_string());
        let index = self.plugins.len() - 1;
        self.call(index, core, Call::Init).await
    }

    pub async fn run_command(
        &mut self,
        plugin: usize,
        core: &mut Core,
        name: &str,
        arg: Option<&str>,
    ) -> Result<(), String> {
        self.call(plugin, core, Call::Command(name, arg)).await
    }

    pub async fn send_event(
        &mut self,
        plugin: usize,
        core: &mut Core,
        event: &Event,
    ) -> Result<(), String> {
        self.call(plugin, core, Call::Event(event)).await
    }

    /// Lend `core` to a plugin for one call.
    async fn call(&mut self, index: usize, core: &mut Core, call: Call<'_>) -> Result<(), String> {
        let plugin = &mut self.plugins[index];
        if plugin.failed {
            return Err(format!(
                "plugin `{}` is disabled after an earlier error",
                plugin.name
            ));
        }
        let state = plugin.store.data_mut();
        state.core = Some(std::mem::take(core));
        state.ticks_left = match call {
            Call::Init => INIT_TICKS,
            Call::Command(..) | Call::Event(_) => CALL_TICKS,
        };
        plugin.store.set_epoch_deadline(1);
        let (bindings, store) = (&plugin.bindings, &mut plugin.store);
        let result = match call {
            Call::Init => bindings.call_init(store).await,
            Call::Command(name, arg) => bindings.call_run_command(store, name, arg).await,
            Call::Event(event) => bindings.call_on_event(store, event).await,
        };
        *core = plugin
            .store
            .data_mut()
            .core
            .take()
            .expect("the core is returned after every call");
        result.map_err(|e| {
            plugin.failed = true;
            format!("plugin `{}` failed and was disabled: {e:#}", plugin.name)
        })
    }
}

/// One call into a plugin.
enum Call<'a> {
    Init,
    Command(&'a str, Option<&'a str>),
    Event(&'a Event),
}
