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
//!
//! - [`buffers`]: the open buffers, and switching between them.
//! - [`registry`]: what plugins have registered (modes, commands, bindings).
//! - [`host_api`]: the functions plugins call.
//! - this module: loading plugins, and lending them the [`Core`] for each call.

mod buffers;
mod compile_cache;
mod processes;
mod snapshots;
mod host_api;
mod registry;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, UpdateDeadline};
use wasmtime_wasi::{FsPerms, WasiCtx};

use crate::components::menu::MenuSpec;
use crate::components::text_editor::TextEditor;
use crate::components::tree_menu::{TreeNode, TreeSpec};
use crate::config::user_config_dir;
use host_api::HostState;
pub use buffers::{Buffer, BufferId};
use snapshots::{Snapshot, SnapshotId};
pub use processes::{ProcessEvent, Processes, Programs};
pub use registry::{Lookup, Registry};

wasmtime::component::bindgen!({
    world: "plugin",
    path: "wit",
    exports: { default: async },
});

use self::arrow::plugin::{
    editor as host_editor, process as host_process, registry as host_registry, types, ui as host_ui,
};
// `Event` is already defined at this level by bindgen, because the world `use`s it.
pub use self::arrow::plugin::types::EventKind;

/// Plugins built from `plugins/` by `build.rs` and embedded in the executable,
/// with what each may reach outside its sandbox: whether it can read the
/// working directory, and which programs it may run. Core loads first so the
/// others can bind its commands.
const BUNDLED: &[(&str, &[u8], bool, &[&str])] = &[
    ("core", include_bytes!(concat!(env!("OUT_DIR"), "/arrow_core.wasm")), false, &[]),
    ("text-tools", include_bytes!(concat!(env!("OUT_DIR"), "/text_tools.wasm")), false, &[]),
    ("files", include_bytes!(concat!(env!("OUT_DIR"), "/files.wasm")), true, &[]),
    ("git", include_bytes!(concat!(env!("OUT_DIR"), "/git.wasm")), false, &["git"]),
];

/// What a plugin may reach outside its sandbox. By default, nothing but
/// stdout and stderr (for `println!` debugging).
#[derive(Clone, Default)]
struct Access {
    /// Read, but not write, the directory Arrow was started in. Inside the
    /// plugin it is the current directory, ".".
    read_working_dir: bool,
    /// Programs it may start through the `process` interface.
    programs: Programs,
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

/// The bundled plugins, compiled once per process (compiling is the slow
/// part), and loaded from `cache` instead when it holds them already (see
/// [`compile_cache`]). The first caller's `cache` is the one used.
fn bundled_components(cache: Option<&Path>) -> &'static [(&'static str, Result<Component, String>)] {
    static COMPILED: OnceLock<Vec<(&'static str, Result<Component, String>)>> = OnceLock::new();
    COMPILED.get_or_init(|| {
        BUNDLED
            .iter()
            .map(|(name, bytes, _, _)| {
                let component = compile_cache::load(engine(), bytes, cache)
                    .map_err(|e| format!("plugin `{name}` failed to compile: {e:#}"));
                (*name, component)
            })
            .collect()
    })
}

/// Where user plugins live: `%APPDATA%\arrow\plugins` on Windows,
/// `$XDG_CONFIG_HOME/arrow/plugins` or `~/.config/arrow/plugins` elsewhere.
pub fn user_plugin_dir() -> Option<PathBuf> {
    user_config_dir().map(|dir| dir.join("plugins"))
}

/// What plugins can see and change: the editor, the registry, and the queues of
/// work they've asked for.
#[derive(Default)]
pub struct Core {
    /// The current buffer, the one plugins edit and the window shows.
    pub editor: TextEditor,
    /// The current buffer's id.
    pub buffer_id: BufferId,
    /// The other open buffers, in no particular order.
    hidden: Vec<Buffer>,
    /// The highest buffer id handed out so far, so ids are never reused.
    last_buffer_id: BufferId,
    /// Undo snapshots plugins have taken, by id (see `snapshots`).
    snapshots: HashMap<SnapshotId, Snapshot>,
    last_snapshot_id: SnapshotId,
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
    /// Programs plugins have started (see `processes`).
    pub processes: Processes,
    /// Commands provided by the app itself rather than a plugin (like `quit`), as
    /// (name, description).
    pub app_commands: Vec<(&'static str, &'static str)>,
}

impl Core {
    /// A core whose first buffer (id 0) is `editor`, with the app's own commands
    /// as (name, description).
    pub fn new(editor: TextEditor, app_commands: Vec<(&'static str, &'static str)>) -> Self {
        Self {
            editor,
            app_commands,
            ..Self::default()
        }
    }

    pub fn is_command(&self, name: &str) -> bool {
        self.registry.command(name).is_some() || self.is_app_command(name)
    }

    pub fn is_app_command(&self, name: &str) -> bool {
        self.app_commands.iter().any(|(n, _)| *n == name)
    }

    /// Every command plugins and the app provide, sorted by name.
    pub(super) fn list_commands(&self) -> Vec<types::CommandInfo> {
        let registry = &self.registry;
        let plugin_commands = registry.commands().map(|(name, command)| types::CommandInfo {
            name: name.to_string(),
            description: command.description.clone(),
            plugin: registry.plugin_name(command.plugin).to_string(),
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
    /// What the caller's `Core` holds while the real one is lent to a plugin.
    ///
    /// Lending moves the `Core` into the plugin's store, and something has to
    /// stay behind in its place. `mem::take` would build a fresh
    /// `Core::default()` for that on every call (an empty text editor: a rope,
    /// a dots vector and a couple of strings), and a key press makes several
    /// calls. Instead one stand-in is built on first use and handed back and
    /// forth, so lending allocates nothing. Nobody reads it: the session waits
    /// for the call to finish.
    stand_in: Option<Core>,
    /// Where compiled plugins are kept between runs. `None` compiles every time
    /// (tests use that, so they never touch the user's cache).
    cache: Option<PathBuf>,
}

impl PluginHost {
    /// A host that keeps compiled plugins in `cache`, if given.
    pub fn new(cache: Option<PathBuf>) -> Self {
        let mut linker = Linker::new(engine());
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("WASI links");
        Plugin::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .expect("plugin interfaces link");
        Self {
            linker,
            plugins: Vec::new(),
            stand_in: None,
            cache,
        }
    }

    pub async fn load_bundled(&mut self, core: &mut Core) -> Vec<String> {
        let mut errors = Vec::new();
        let access = BUNDLED.iter().map(|(_, _, read_working_dir, programs)| Access {
            read_working_dir: *read_working_dir,
            programs: Programs::from_list(programs.iter().map(|p| p.to_string()).collect()),
        });
        let components = bundled_components(self.cache.as_deref());
        for ((name, component), access) in components.iter().zip(access) {
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

    /// Load every `*.wasm` in the user plugin directory, in name order, with
    /// the programs `plugins.toml` allows each to run.
    pub async fn load_user_plugins(&mut self, core: &mut Core) -> Vec<String> {
        let (Some(config), Some(dir)) = (user_config_dir(), user_plugin_dir()) else {
            return Vec::new();
        };
        let mut errors = Vec::new();
        let mut permissions = processes::load_permissions(&config).unwrap_or_else(|error| {
            errors.push(error);
            HashMap::new()
        });
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut paths: Vec<_> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "wasm"))
            .collect();
        paths.sort();
        for path in &paths {
            let name = plugin_name(path);
            let programs = permissions.remove(&name).unwrap_or_default();
            if let Err(error) = self.load_file(path, core, programs).await {
                errors.push(error);
            }
        }
        errors
    }

    async fn load_file(&mut self, path: &Path, core: &mut Core, programs: Programs) -> Result<(), String> {
        let name = plugin_name(path);
        let bytes = std::fs::read(path).map_err(|e| format!("plugin `{name}`: {e}"))?;
        let component = compile_cache::load(engine(), &bytes, self.cache.as_deref())
            .map_err(|e| format!("plugin `{name}`: {e:#}"))?;
        // User plugins can't read files, and run only what the user allowed.
        let access = Access {
            read_working_dir: false,
            programs,
        };
        self.load(&name, &component, core, access).await
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
        if access.read_working_dir {
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
                programs: access.programs,
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
        core.registry.add_plugin(name);
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
        // Swap the real core out for the stand-in (built once, the first time).
        let stand_in = self.stand_in.take().unwrap_or_default();
        let state = plugin.store.data_mut();
        state.core = Some(std::mem::replace(core, stand_in));
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
        // Swap it back, and keep the stand-in for next time.
        let lent = plugin
            .store
            .data_mut()
            .core
            .take()
            .expect("the core is returned after every call");
        self.stand_in = Some(std::mem::replace(core, lent));
        result.map_err(|e| {
            plugin.failed = true;
            // Nothing will handle their output any more.
            core.processes.stop_all_of(index);
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

/// A user plugin's name: its file name without `.wasm`. `plugins.toml` uses it.
fn plugin_name(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
