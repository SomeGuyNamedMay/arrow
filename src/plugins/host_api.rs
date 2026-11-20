//! The host side of the WIT interfaces: what plugins call to read and change
//! the editor, register commands and bindings, and open menus.

use wasmtime::component::ResourceTable;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use super::registry::PluginCommand;
use super::{Core, EventKind, host_editor, host_registry, host_ui, types};
use crate::components::menu::{MenuItem, MenuSpec};
use crate::components::text_editor::Selection;
use crate::components::tree_menu::{TreeNode, TreeSpec};

/// A plugin store's data. `core` is only present during a call.
pub struct HostState {
    pub(super) core: Option<Core>,
    /// This plugin's index.
    pub(super) plugin: usize,
    /// Epoch ticks the current call may still run for.
    pub(super) ticks_left: u64,
    pub(super) wasi: WasiCtx,
    pub(super) table: ResourceTable,
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
        self.core().registry.define_mode(name, fallback);
    }

    fn register_command(&mut self, name: String, description: String, takes_char: bool) {
        let plugin = self.plugin;
        let command = PluginCommand {
            plugin,
            description,
            takes_char,
        };
        self.core().registry.register_command(name, command);
    }

    fn bind_key(&mut self, mode: String, keys: String, command: String) -> Result<(), String> {
        let registry = &mut self.core().registry;
        if !registry.has_mode(&mode) {
            return Err(format!("no such mode: {mode}"));
        }
        registry.bind_key(&mode, &keys, &command)
    }

    fn name_prefix(&mut self, mode: String, keys: String, label: String) -> Result<(), String> {
        self.core().registry.name_prefix(mode, &keys, label)
    }

    fn subscribe(&mut self, event: EventKind) {
        let plugin = self.plugin;
        self.core().registry.subscribe(plugin, event);
    }

    fn list_commands(&mut self) -> Vec<types::CommandInfo> {
        self.core().list_commands()
    }

    fn list_bindings(&mut self) -> Vec<types::BindingInfo> {
        self.core()
            .registry
            .bindings()
            .map(|(mode, keys, command)| types::BindingInfo {
                mode: mode.to_string(),
                keys,
                command: command.to_string(),
            })
            .collect()
    }
}
