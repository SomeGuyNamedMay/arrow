use super::api;
use super::{PYTHON_ACTIONS, PythonAction};
use crate::components::text_editor::Dot;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule, PyString, PyTuple};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;

static PYTHON_INIT_LOCK: Mutex<()> = Mutex::new(());

const DEFAULT_COMMANDS: &str = include_str!("../../../python/defaults/commands.py");
const DEFAULT_VERTICO: &str = include_str!("../../../python/defaults/vertico.py");
const DEFAULT_KEYBINDINGS: &str = include_str!("../../../python/defaults/keybindings.py");

pub struct PythonRuntime {
    module: Py<PyModule>,
    commands: BTreeMap<String, String>,
}

impl PythonRuntime {
    pub fn take_actions() -> Vec<PythonAction> {
        PYTHON_ACTIONS.with(|actions| std::mem::take(&mut *actions.borrow_mut()))
    }

    pub fn new() -> Result<Self, String> {
        let _initialization = PYTHON_INIT_LOCK
            .lock()
            .map_err(|_| "Python runtime initialization lock is poisoned".to_string())?;
        Python::attach(|py| {
            let code = CString::new(api::ARROW_PYTHON_API).unwrap();
            let file = CString::new("arrow.py").unwrap();
            let name = CString::new("arrow").unwrap();
            let module = PyModule::from_code(py, code.as_c_str(), file.as_c_str(), name.as_c_str())
                .map_err(|error| format!("Failed to create Arrow Python API: {error}"))?;
            let modules = py
                .import("sys")
                .and_then(|sys| sys.getattr("modules"))
                .map_err(|error| format!("Failed to access Python modules: {error}"))?
                .cast_into::<PyDict>()
                .map_err(|error| format!("Failed to access Python modules: {error}"))?;
            modules
                .set_item("arrow", module.clone())
                .map_err(|error| format!("Failed to register Arrow Python module: {error}"))?;
            api::register(py, &module)?;

            let default_commands = CString::new(DEFAULT_COMMANDS).unwrap();
            py.run(default_commands.as_c_str(), Some(&module.dict()), None)
                .map_err(|error| {
                    let traceback = error.traceback(py).map_or_else(
                        || String::new(),
                        |traceback| traceback.format().unwrap_or_default(),
                    );
                    format!("Failed to load default Python commands: {error}\n{traceback}")
                })?;
            let default_vertico = CString::new(DEFAULT_VERTICO).unwrap();
            py.run(default_vertico.as_c_str(), Some(&module.dict()), None)
                .map_err(|error| {
                    let traceback = error.traceback(py).map_or_else(
                        || String::new(),
                        |traceback| traceback.format().unwrap_or_default(),
                    );
                    format!("Failed to load default Vertico Python module: {error}\n{traceback}")
                })?;
            let default_keybindings = CString::new(DEFAULT_KEYBINDINGS).unwrap();
            py.run(default_keybindings.as_c_str(), Some(&module.dict()), None)
                .map_err(|error| {
                    let traceback = error.traceback(py).map_or_else(
                        || String::new(),
                        |traceback| traceback.format().unwrap_or_default(),
                    );
                    format!("Failed to load default Python keybindings: {error}\n{traceback}")
                })?;

            let init_path = super::config_directory().join("init.py");
            if init_path.is_file() {
                let source = std::fs::read_to_string(&init_path)
                    .map_err(|error| format!("Failed to read {}: {error}", init_path.display()))?;
                let source = CString::new(source).unwrap();
                py.run(source.as_c_str(), Some(&module.dict()), None)
                    .map_err(|error| format!("Python startup error: {error}"))?;
            }

            Self::collect_commands(module.clone().into()).map(|commands| Self {
                module: module.unbind(),
                commands,
            })
        })
    }

    fn collect_commands(module: Py<PyModule>) -> Result<BTreeMap<String, String>, String> {
        Python::attach(|py| {
            let module = module.bind(py);
            let commands = module
                .getattr("commands")
                .and_then(|value| {
                    value
                        .cast::<PyDict>()
                        .map_err(Into::into)
                        .map(|dict| dict.clone())
                })
                .map_err(|error| format!("Arrow Python commands must be a dictionary: {error}"))?;

            let mut names = BTreeMap::new();
            for name in commands.keys() {
                let name = name
                    .extract::<String>()
                    .map_err(|error| error.to_string())?;
                names.insert(name.clone(), name);
            }
            Ok(names)
        })
    }

    #[cfg(test)]
    pub fn command_names(&self) -> Vec<String> {
        self.commands.keys().cloned().collect()
    }

    pub fn set_editor_dots(&self, dots: &[Dot]) -> Result<(), String> {
        Python::attach(|py| {
            let module = self.module.bind(py);
            api::publish_editor_dots(module, dots.to_vec()).map_err(|error| error.to_string())
        })
    }

    pub fn set_editor_text(&self, text: &str) -> Result<(), String> {
        super::EDITOR_TEXT.with(|stored| *stored.borrow_mut() = text.to_string());
        Ok(())
    }

    pub fn command_for_key(&self, mode: &str, key: &str) -> Option<String> {
        Python::attach(|py| {
            let module = self.module.bind(py);
            let keybindings = module.getattr("keybindings").ok()?;
            let keybindings = keybindings.cast::<PyDict>().ok()?;
            let mode_bindings = keybindings.get_item(mode).ok()??;
            let mode_bindings = mode_bindings.cast::<PyDict>().ok()?;
            mode_bindings.get_item(key).ok()??.extract::<String>().ok()
        })
    }

    pub fn execute(&self, name: &str) -> Result<(), String> {
        if !self.commands.contains_key(name) {
            return Err(format!("Unknown Python command: {name}"));
        }

        Python::attach(|py| {
            let module = self.module.bind(py);
            let commands = module
                .getattr("commands")
                .and_then(|value| {
                    value
                        .cast::<PyDict>()
                        .map_err(Into::into)
                        .map(|dict| dict.clone())
                })
                .map_err(|error| error.to_string())?;
            let function = commands
                .get_item(name)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("Unknown Python command: {name}"))?;
            function
                .call0()
                .map_err(|error| format!("Python command '{name}' failed: {error}"))?;
            Ok(())
        })
    }

    pub fn execute_with_args(&self, name: &str, arguments: &[String]) -> Result<(), String> {
        if !self.commands.contains_key(name) {
            return Err(format!("Unknown Python command: {name}"));
        }

        Python::attach(|py| {
            let module = self.module.bind(py);
            let commands = module
                .getattr("commands")
                .map_err(|error| error.to_string())?;
            let commands = commands
                .cast::<PyDict>()
                .map_err(|error| error.to_string())?;
            let function = commands
                .get_item(name)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("Unknown Python command: {name}"))?;
            let arguments = arguments
                .iter()
                .map(|argument| PyString::new(py, argument).into_any().unbind())
                .collect::<Vec<_>>();
            let arguments = PyTuple::new(py, arguments).map_err(|error| error.to_string())?;
            function
                .call1(arguments)
                .map_err(|error| format!("Python command '{name}' failed: {error}"))?;
            Ok(())
        })
    }

    pub fn run_hook(&self, name: &str) -> Result<(), String> {
        Python::attach(|py| {
            let module = self.module.bind(py);
            let hooks = module
                .getattr("hooks")
                .and_then(|value| {
                    value
                        .cast::<PyDict>()
                        .map_err(Into::into)
                        .map(|dict| dict.clone())
                })
                .map_err(|error| error.to_string())?;
            let Some(hooks) = hooks.get_item(name).map_err(|error| error.to_string())? else {
                return Ok(());
            };
            let hooks = hooks.cast::<PyList>().map_err(|error| error.to_string())?;
            for hook in hooks.iter() {
                hook.call0()
                    .map_err(|error| format!("Python hook '{name}' failed: {error}"))?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::python::PythonAction;
    use crate::components::text_editor::SelectionRange;

    fn run_dot_command(runtime: &PythonRuntime, command: &str, dots: &[Dot]) -> Vec<Dot> {
        runtime.set_editor_dots(dots).unwrap();
        runtime.execute(command).unwrap();
        let actions = PythonRuntime::take_actions();
        let [PythonAction::ReplaceDots(dots)] = actions.as_slice() else {
            panic!("'{command}' should update editor dots");
        };
        dots.clone()
    }

    #[test]
    fn default_python_commands_are_registered() {
        let runtime = PythonRuntime::new().unwrap();
        let names = runtime.command_names();

        assert!(names.contains(&"about-arrow".to_string()));
        assert!(names.contains(&"current-directory".to_string()));
        assert!(names.contains(&"list-directory".to_string()));
        assert!(names.contains(&"default-python-commands".to_string()));
        assert!(names.contains(&"open-readme".to_string()));
        assert!(names.contains(&"write-output-file".to_string()));
    }

    #[test]
    fn default_python_keybindings_execute() {
        let runtime = PythonRuntime::new().unwrap();

        assert_eq!(
            runtime.command_for_key("normal", "i"),
            Some("insert-mode".to_string())
        );
        assert_eq!(
            runtime.command_for_key("insert", "escape"),
            Some("normal-mode".to_string())
        );

        runtime.execute("insert-mode").unwrap();
        assert_eq!(
            PythonRuntime::take_actions(),
            vec![PythonAction::EnterInsertMode]
        );
    }

    #[test]
    fn toggle_vertico_uses_execute_command() {
        let runtime = PythonRuntime::new().unwrap();
        assert_eq!(
            runtime.command_for_key("normal", "space"),
            Some("toggle-vertico".to_string())
        );
        runtime.execute("toggle-vertico").unwrap();

        let actions = PythonRuntime::take_actions();
        let [
            PythonAction::ShowVertico {
                title,
                prompt,
                action,
                items,
            },
        ] = actions.as_slice()
        else {
            panic!("toggle-vertico should open the command palette");
        };
        assert_eq!(title, "Commands");
        assert_eq!(prompt, "M-x");
        assert_eq!(action, "execute-command");
        assert!(items.iter().any(|(name, _)| name == "toggle-vertico"));

        runtime
            .execute_with_args("execute-command", &["about-arrow".to_string()])
            .unwrap();
        assert_eq!(
            PythonRuntime::take_actions(),
            vec![PythonAction::ExecuteCommand("about-arrow".to_string())]
        );
    }

    #[test]
    fn normal_python_commands_execute() {
        let runtime = PythonRuntime::new().unwrap();
        for command in [
            "move-left",
            "move-down",
            "move-up",
            "move-right",
            "move-next-word",
            "move-previous-word",
            "select-line",
            "append-mode",
            "add-cursor-below",
            "keep-primary-cursor",
            "delete",
            "paste",
            "clear-selection",
        ] {
            PythonRuntime::take_actions();
            runtime
                .execute(command)
                .unwrap_or_else(|error| panic!("Python command '{command}' failed: {error}"));
        }
    }

    #[test]
    fn editor_dots_are_exposed_to_python() {
        let runtime = PythonRuntime::new().unwrap();
        runtime
            .set_editor_dots(&[
                Dot {
                    cursor: 4,
                    selection: None,
                    is_focused: false,
                },
                Dot {
                    cursor: 9,
                    selection: Some(SelectionRange { start: 7, end: 10 }),
                    is_focused: true,
                },
            ])
            .unwrap();

        Python::attach(|py| {
            let module = runtime.module.bind(py);
            assert_eq!(
                module
                    .getattr("dots")
                    .unwrap()
                    .call0()
                    .unwrap()
                    .len()
                    .unwrap(),
                2
            );
            let dots = module.getattr("dots").unwrap().call0().unwrap();
            assert!(
                !dots
                    .get_item(0)
                    .unwrap()
                    .get_item("is_focused")
                    .unwrap()
                    .extract::<bool>()
                    .unwrap()
            );
            assert!(
                dots.get_item(1)
                    .unwrap()
                    .get_item("is_focused")
                    .unwrap()
                    .extract::<bool>()
                    .unwrap()
            );
        });
    }

    #[test]
    fn focus_helpers_manage_multiple_dots() {
        let runtime = PythonRuntime::new().unwrap();
        runtime
            .set_editor_dots(&[
                Dot {
                    cursor: 1,
                    selection: None,
                    is_focused: false,
                },
                Dot {
                    cursor: 4,
                    selection: None,
                    is_focused: true,
                },
                Dot {
                    cursor: 8,
                    selection: None,
                    is_focused: false,
                },
            ])
            .unwrap();

        Python::attach(|py| {
            let module = runtime.module.bind(py);
            let focused = module.getattr("focused_dots").unwrap().call0().unwrap();
            assert_eq!(focused.len().unwrap(), 1);
            assert_eq!(
                focused
                    .get_item(0)
                    .unwrap()
                    .get_item("cursor")
                    .unwrap()
                    .extract::<usize>()
                    .unwrap(),
                4
            );
            module
                .getattr("set_dot_focused")
                .unwrap()
                .call1((0, true))
                .unwrap();
        });

        let actions = PythonRuntime::take_actions();
        let [PythonAction::ReplaceDots(dots)] = actions.as_slice() else {
            panic!("set_dot_focused should update dot focus state");
        };
        assert_eq!(
            dots.iter().map(|dot| dot.is_focused).collect::<Vec<_>>(),
            vec![true, true, false]
        );

        Python::attach(|py| {
            let module = runtime.module.bind(py);
            module.getattr("unfocus_all_dots").unwrap().call0().unwrap();
        });
        let actions = PythonRuntime::take_actions();
        let [PythonAction::ReplaceDots(dots)] = actions.as_slice() else {
            panic!("unfocus_all_dots should update every dot");
        };
        assert!(dots.iter().all(|dot| !dot.is_focused));

        Python::attach(|py| {
            let module = runtime.module.bind(py);
            module.getattr("focus_all_dots").unwrap().call0().unwrap();
        });
        let actions = PythonRuntime::take_actions();
        let [PythonAction::ReplaceDots(dots)] = actions.as_slice() else {
            panic!("focus_all_dots should update every dot");
        };
        assert!(dots.iter().all(|dot| dot.is_focused));
    }

    #[test]
    fn add_cursor_below_creates_an_unfocused_dot() {
        let runtime = PythonRuntime::new().unwrap();
        runtime.set_editor_text("one\ntwo").unwrap();
        runtime
            .set_editor_dots(&[Dot {
                cursor: 1,
                selection: None,
                is_focused: true,
            }])
            .unwrap();

        runtime.execute("add-cursor-below").unwrap();

        let actions = PythonRuntime::take_actions();
        let [PythonAction::ReplaceDots(dots)] = actions.as_slice() else {
            panic!("add-cursor-below should replace the editor dots");
        };
        assert_eq!(
            dots.iter().map(|dot| dot.is_focused).collect::<Vec<_>>(),
            vec![true, false]
        );
    }

    #[test]
    fn movement_and_selection_commands_ignore_unfocused_dots() {
        let runtime = PythonRuntime::new().unwrap();
        runtime.set_editor_text("one\ntwo").unwrap();
        let dots = [
            Dot {
                cursor: 1,
                selection: Some(SelectionRange { start: 0, end: 2 }),
                is_focused: true,
            },
            Dot {
                cursor: 5,
                selection: Some(SelectionRange { start: 4, end: 6 }),
                is_focused: false,
            },
        ];

        let moved = run_dot_command(&runtime, "move-right", &dots);
        assert_eq!(moved[0].cursor, 2);
        assert_eq!(moved[0].selection, None);
        assert_eq!(moved[1], dots[1]);

        let moved = run_dot_command(&runtime, "move-down", &dots);
        assert_eq!(moved[0].cursor, 5);
        assert_eq!(moved[0].selection, None);
        assert_eq!(moved[1], dots[1]);

        let selected = run_dot_command(&runtime, "select-line", &dots);
        assert_eq!(selected[0].cursor, 3);
        assert_eq!(
            selected[0].selection,
            Some(SelectionRange { start: 0, end: 3 })
        );
        assert_eq!(selected[1], dots[1]);

        let cleared = run_dot_command(&runtime, "clear-selection", &dots);
        assert_eq!(cleared[0].selection, None);
        assert_eq!(cleared[1], dots[1]);
    }

    #[test]
    fn bracket_keys_cycle_the_focused_dot() {
        let runtime = PythonRuntime::new().unwrap();
        let dots = [
            Dot {
                cursor: 1,
                selection: None,
                is_focused: false,
            },
            Dot {
                cursor: 4,
                selection: None,
                is_focused: true,
            },
            Dot {
                cursor: 8,
                selection: None,
                is_focused: false,
            },
        ];

        assert_eq!(
            runtime.command_for_key("normal", "["),
            Some("focus-previous-dot".to_string())
        );
        assert_eq!(
            runtime.command_for_key("normal", "]"),
            Some("focus-next-dot".to_string())
        );

        let previous = run_dot_command(&runtime, "focus-previous-dot", &dots);
        assert_eq!(
            previous
                .iter()
                .map(|dot| dot.is_focused)
                .collect::<Vec<_>>(),
            vec![true, false, false]
        );

        let next = run_dot_command(&runtime, "focus-next-dot", &dots);
        assert_eq!(
            next.iter().map(|dot| dot.is_focused).collect::<Vec<_>>(),
            vec![false, false, true]
        );
    }

    #[test]
    fn set_dots_preserves_focus_when_not_specified() {
        let runtime = PythonRuntime::new().unwrap();
        runtime
            .set_editor_dots(&[
                Dot {
                    cursor: 1,
                    selection: None,
                    is_focused: true,
                },
                Dot {
                    cursor: 4,
                    selection: None,
                    is_focused: false,
                },
            ])
            .unwrap();

        Python::attach(|py| {
            let module = runtime.module.bind(py);
            let first = PyDict::new(py);
            first.set_item("cursor", 2).unwrap();
            first.set_item("selection", py.None()).unwrap();
            let second = PyDict::new(py);
            second.set_item("cursor", 5).unwrap();
            second.set_item("selection", py.None()).unwrap();
            let new_dots = PyList::new(py, [first, second]).unwrap();
            module
                .getattr("set_dots")
                .unwrap()
                .call1((new_dots,))
                .unwrap();
        });
        let actions = PythonRuntime::take_actions();
        let [PythonAction::ReplaceDots(dots)] = actions.as_slice() else {
            panic!("set_dots should replace dot positions");
        };
        assert_eq!(
            dots.iter().map(|dot| dot.is_focused).collect::<Vec<_>>(),
            vec![true, false]
        );
    }
}
