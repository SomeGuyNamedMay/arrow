use super::api;
use super::{PYTHON_ACTIONS, PythonAction};
use crate::components::syntax::{AstNode, LanguageServerConfig, SyntaxLanguageConfig};
use crate::components::text_editor::Dot;
use iced::Color;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule, PyString, PyTuple};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;

static PYTHON_INIT_LOCK: Mutex<()> = Mutex::new(());

const DEFAULT_COMMANDS: &str = include_str!("../../../python/defaults/commands.py");
const DEFAULT_SYNTAX: &str = include_str!("../../../python/defaults/syntax.py");
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
            let default_syntax = CString::new(DEFAULT_SYNTAX).unwrap();
            py.run(default_syntax.as_c_str(), Some(&module.dict()), None)
                .map_err(|error| {
                    let traceback = error.traceback(py).map_or_else(
                        || String::new(),
                        |traceback| traceback.format().unwrap_or_default(),
                    );
                    format!("Failed to load default syntax configuration: {error}\n{traceback}")
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

    pub fn syntax_languages(&self) -> Result<Vec<SyntaxLanguageConfig>, String> {
        Python::attach(|py| {
            let module = self.module.bind(py);
            let languages = module
                .getattr("syntax_languages")
                .map_err(|error| format!("Python syntax_languages must be a dictionary: {error}"))?
                .cast_into::<PyDict>()
                .map_err(|error| {
                    format!("Python syntax_languages must be a dictionary: {error}")
                })?;

            languages
                .iter()
                .map(|(_, value)| {
                    let language = value
                        .cast::<PyDict>()
                        .map_err(|error| format!("Invalid Python language definition: {error}"))?;
                    let get_string = |key: &str| -> Result<String, String> {
                        language
                            .get_item(key)
                            .map_err(|error| error.to_string())?
                            .ok_or_else(|| format!("Language definition is missing '{key}'"))?
                            .extract::<String>()
                            .map_err(|error| format!("Language field '{key}' must be a string: {error}"))
                    };
                    let name = get_string("name")?;
                    let parser = language
                        .get_item("parser")
                        .map_err(|error| error.to_string())?
                        .filter(|value| !value.is_none());
                    if parser.as_ref().is_some_and(|parser| !parser.is_callable()) {
                        return Err(format!("Language '{name}' parser must be callable"));
                    }
                    let language_server = language
                        .get_item("language_server")
                        .map_err(|error| error.to_string())?
                        .filter(|value| !value.is_none())
                        .map(|value| {
                            let server = value
                                .cast_into::<PyDict>()
                                .map_err(|error| format!("Language '{name}' language_server must be a dictionary: {error}"))?;
                            let Some(command_value) = server
                                .get_item("command")
                                .map_err(|error| error.to_string())?
                            else {
                                return Err(format!("Language '{name}' language server is missing 'command'"));
                            };
                            let command = command_value
                                .extract::<String>()
                                .map_err(|error| format!("Language '{name}' language server command must be a string: {error}"))?;
                            let arguments = server
                                .get_item("args")
                                .map_err(|error| error.to_string())?
                                .map(|value| value.extract::<Vec<String>>())
                                .transpose()
                                .map_err(|error| format!("Language '{name}' language server args must be strings: {error}"))?
                                .unwrap_or_default();
                            let root_markers = server
                                .get_item("root_markers")
                                .map_err(|error| error.to_string())?
                                .map(|value| value.extract::<Vec<String>>())
                                .transpose()
                                .map_err(|error| format!("Language '{name}' root markers must be strings: {error}"))?
                                .unwrap_or_default();
                            Ok::<_, String>(LanguageServerConfig {
                                command,
                                arguments,
                                root_markers,
                            })
                        })
                        .transpose()?;
                    let extensions = language
                        .get_item("extensions")
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| format!("Language '{name}' is missing 'extensions'"))?
                        .extract::<Vec<String>>()
                        .map_err(|error| format!("Language '{name}' extensions must be strings: {error}"))?;
                    let colors = language
                        .get_item("highlight_colors")
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| format!("Language '{name}' is missing 'highlight_colors'"))?
                        .cast_into::<PyDict>()
                        .map_err(|error| format!("Language '{name}' highlight_colors must be a dictionary: {error}"))?;
                    let mut highlight_colors = BTreeMap::new();
                    for (capture, value) in colors.iter() {
                        let capture = capture
                            .extract::<String>()
                            .map_err(|error| format!("Language '{name}' capture name must be a string: {error}"))?;
                        let values = value
                            .cast::<PyTuple>()
                            .map_err(|error| format!("Color for capture '{capture}' must be a tuple: {error}"))?;
                        if values.len() != 4 {
                            return Err(format!("Color for capture '{capture}' must contain red, green, blue, and priority"));
                        }
                        let red = values.get_item(0).and_then(|item| item.extract::<f32>())
                            .map_err(|error| format!("Invalid red channel for capture '{capture}': {error}"))?;
                        let green = values.get_item(1).and_then(|item| item.extract::<f32>())
                            .map_err(|error| format!("Invalid green channel for capture '{capture}': {error}"))?;
                        let blue = values.get_item(2).and_then(|item| item.extract::<f32>())
                            .map_err(|error| format!("Invalid blue channel for capture '{capture}': {error}"))?;
                        let priority = values.get_item(3).and_then(|item| item.extract::<u8>())
                            .map_err(|error| format!("Invalid priority for capture '{capture}': {error}"))?;
                        highlight_colors
                            .insert(capture, (Color::from_rgb(red, green, blue), priority));
                    }
                    let text_object_kinds = language
                        .get_item("text_object_kinds")
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| format!("Language '{name}' is missing 'text_object_kinds'"))?
                        .extract::<Vec<String>>()
                        .map_err(|error| format!("Language '{name}' text_object_kinds must be strings: {error}"))?;
                    Ok(SyntaxLanguageConfig {
                        name,
                        extensions,
                        highlight_colors,
                        text_object_kinds,
                        language_server,
                    })
                })
                .collect()
        })
    }

    pub fn parse_syntax(
        &self,
        language_name: &str,
        source: &str,
    ) -> Result<Option<AstNode>, String> {
        Python::attach(|py| {
            let module = self.module.bind(py);
            let languages = module.getattr("syntax_languages").map_err(|error| {
                format!("Python syntax_languages must be a dictionary: {error}")
            })?;
            let languages = languages.cast_into::<PyDict>().map_err(|error| {
                format!("Python syntax_languages must be a dictionary: {error}")
            })?;
            let Some(language) = languages
                .get_item(language_name)
                .map_err(|error| error.to_string())?
            else {
                return Err(format!("No Python syntax language named '{language_name}'"));
            };
            let language = language.cast_into::<PyDict>().map_err(|error| {
                format!("Language '{language_name}' must be a dictionary: {error}")
            })?;
            let Some(parser) = language
                .get_item("parser")
                .map_err(|error| error.to_string())?
                .filter(|parser| !parser.is_none())
            else {
                return Ok(None);
            };
            if !parser.is_callable() {
                return Err(format!(
                    "Language '{language_name}' parser must be callable"
                ));
            }
            let result = parser
                .call1((source,))
                .map_err(|error| format!("Python parser for '{language_name}' failed: {error}"))?;
            if result.is_none() {
                return Ok(None);
            }
            parse_ast_node(&result, source.chars().count(), 0, None).map(Some)
        })
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

fn parse_ast_node(
    value: &Bound<'_, PyAny>,
    source_length: usize,
    depth: usize,
    parent_range: Option<(usize, usize)>,
) -> Result<AstNode, String> {
    if depth > 256 {
        return Err("Python syntax AST exceeds the maximum nesting depth".to_string());
    }
    let node = value
        .cast::<PyDict>()
        .map_err(|error| format!("Python parser AST nodes must be dictionaries: {error}"))?;
    let get_required = |key: &str| -> Result<Bound<'_, PyAny>, String> {
        node.get_item(key)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Python parser AST node is missing '{key}'"))
    };
    let kind = get_required("kind")?
        .extract::<String>()
        .map_err(|error| format!("Python parser AST node kind must be a string: {error}"))?;
    if kind.is_empty() {
        return Err("Python parser AST node kind must not be empty".to_string());
    }
    let start = get_required("start")?.extract::<usize>().map_err(|error| {
        format!("Python parser AST node start must be a nonnegative integer: {error}")
    })?;
    let end = get_required("end")?.extract::<usize>().map_err(|error| {
        format!("Python parser AST node end must be a nonnegative integer: {error}")
    })?;
    if start > end || end > source_length {
        return Err(format!(
            "Python parser AST node '{kind}' has invalid range {start}..{end} for {source_length} characters"
        ));
    }
    if parent_range
        .is_some_and(|(parent_start, parent_end)| start < parent_start || end > parent_end)
    {
        return Err(format!(
            "Python parser AST node '{kind}' is outside its parent range"
        ));
    }
    let capture = node
        .get_item("capture")
        .map_err(|error| error.to_string())?
        .filter(|value| !value.is_none())
        .map(|value| {
            value.extract::<String>().map_err(|error| {
                format!("Python parser AST node '{kind}' capture must be a string: {error}")
            })
        })
        .transpose()?;
    let children = node
        .get_item("children")
        .map_err(|error| error.to_string())?
        .map(|children| {
            children
                .cast_into::<PyList>()
                .map_err(|error| {
                    format!("Python parser AST node '{kind}' children must be a list: {error}")
                })?
                .iter()
                .map(|child| parse_ast_node(&child, source_length, depth + 1, Some((start, end))))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();

    Ok(AstNode {
        kind,
        start,
        end,
        capture,
        children,
    })
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
        assert_eq!(runtime.command_for_key("insert", "enter"), None);
        assert_eq!(
            runtime.command_for_key("normal", "f"),
            Some("select-function".to_string())
        );
        assert_eq!(
            runtime.command_for_key("vertico", "tab"),
            Some("vertico-down".to_string())
        );

        runtime.execute("insert-mode").unwrap();
        assert_eq!(
            PythonRuntime::take_actions(),
            vec![PythonAction::EnterInsertMode]
        );
    }

    #[test]
    fn default_python_syntax_configuration_registers_rust_and_python() {
        let runtime = PythonRuntime::new().unwrap();
        let languages = runtime.syntax_languages().unwrap();

        assert_eq!(languages.len(), 2);
        assert!(languages.iter().any(|language| {
            language.name == "rust"
                && language.extensions == ["rs"]
                && language
                    .language_server
                    .as_ref()
                    .is_some_and(|server| server.command == "rust-analyzer")
                && !language.highlight_colors.is_empty()
        }));
        assert!(languages.iter().any(|language| {
            language.name == "python"
                && language.extensions == ["py", "pyi"]
                && language.language_server.as_ref().is_some_and(|server| {
                    server.command == "pyright-langserver" && server.arguments == ["--stdio"]
                })
        }));
    }

    #[test]
    fn python_can_register_language_parser_ast_and_text_object_kinds() {
        let runtime = PythonRuntime::new().unwrap();
        Python::attach(|py| {
            let module = runtime.module.bind(py);
            let source = CString::new(
                "def parse_test(source):\n    return ast_node('module', 0, len(source), [\n        ast_node('function', 0, len(source), [\n            ast_node('keyword', 0, 2, capture='keyword')\n        ])\n    ])\nregister_language('rust-test', ('rsx',), parser=parse_test,\n    highlight_colors={'keyword': (0.7, 0.4, 0.9, 2)},\n    text_object_kinds=('function',))",
            )
            .unwrap();
            py.run(source.as_c_str(), Some(&module.dict()), None)
                .unwrap();
        });

        let languages = runtime.syntax_languages().unwrap();
        let custom = languages
            .iter()
            .find(|language| language.name == "rust-test")
            .unwrap();
        assert_eq!(custom.extensions, ["rsx"]);
        assert!(custom.highlight_colors.contains_key("keyword"));
        assert_eq!(custom.text_object_kinds, ["function"]);

        let tree = runtime.parse_syntax("rust-test", "fn").unwrap().unwrap();
        assert_eq!(tree.kind, "module");
        assert_eq!(tree.children[0].kind, "function");
        assert_eq!(
            tree.children[0].children[0].capture.as_deref(),
            Some("keyword")
        );
    }

    fn captured_text(tree: &AstNode, source: &str, capture: &str) -> Vec<String> {
        let mut texts = Vec::new();
        let mut stack = vec![tree];
        while let Some(node) = stack.pop() {
            if node.capture.as_deref() == Some(capture) {
                texts.push(
                    source
                        .chars()
                        .skip(node.start)
                        .take(node.end - node.start)
                        .collect(),
                );
            }
            stack.extend(&node.children);
        }
        texts
    }

    fn text_object_at(tree: &AstNode, source: &str, needle: &str) -> Option<String> {
        let cursor = source[..source.find(needle).unwrap()].chars().count();
        tree.enclosing_text_object(cursor, &["function".to_string()])
            .map(|(start, end)| source.chars().skip(start).take(end - start).collect())
    }

    #[test]
    fn default_rust_parser_highlights_and_finds_functions() {
        let runtime = PythonRuntime::new().unwrap();
        let source = "// café\npub(crate) fn greet(name: &'a str) -> u8 {\n    let c = '\\n';\n    println!(\"hi {}\", name);\n    fn inner() -> i32 { MAX + 1 }\n    let s = r#\"fn fake() {\"#;\n    0\n}\n\ntrait Shape { fn area(&self) -> f64; }\n";
        let tree = runtime.parse_syntax("rust", source).unwrap().unwrap();

        assert_eq!(captured_text(&tree, source, "comment"), ["// café"]);
        assert!(captured_text(&tree, source, "keyword").contains(&"fn".to_string()));
        assert_eq!(captured_text(&tree, source, "macro"), ["println"]);
        assert_eq!(captured_text(&tree, source, "character"), ["'\\n'"]);
        assert_eq!(captured_text(&tree, source, "lifetime"), ["'a"]);
        assert_eq!(captured_text(&tree, source, "constant"), ["MAX"]);
        let mut functions = captured_text(&tree, source, "function");
        functions.sort();
        assert_eq!(functions, ["area", "greet", "inner"]);
        let mut strings = captured_text(&tree, source, "string");
        strings.sort();
        assert_eq!(strings, ["\"hi {}\"", "r#\"fn fake() {\"#"]);

        assert_eq!(
            text_object_at(&tree, source, "MAX").as_deref(),
            Some("fn inner() -> i32 { MAX + 1 }")
        );
        let outer = text_object_at(&tree, source, "println").unwrap();
        assert!(outer.starts_with("pub(crate) fn greet"));
        assert!(outer.ends_with("    0\n}"));
        assert_eq!(
            text_object_at(&tree, source, "area").as_deref(),
            Some("fn area(&self) -> f64;")
        );
        assert_eq!(text_object_at(&tree, source, "trait"), None);
    }

    #[test]
    fn default_python_parser_highlights_and_finds_functions() {
        let runtime = PythonRuntime::new().unwrap();
        let source = "import os\n\n@cache\nasync def outer(a, b=(1,\n2)):\n    \"\"\"Doc\nat column zero\"\"\"\n    label = 'café'  # note\n    total = a + \\\nb\n    def inner():\n        return label\n# dedented comment\n    return LIMIT, Point(1), None\n\nx = 1\n";
        let tree = runtime.parse_syntax("python", source).unwrap().unwrap();

        let mut comments = captured_text(&tree, source, "comment");
        comments.sort();
        assert_eq!(comments, ["# dedented comment", "# note"]);
        let mut functions = captured_text(&tree, source, "function");
        functions.sort();
        assert_eq!(functions, ["inner", "outer"]);
        let mut constants = captured_text(&tree, source, "constant");
        constants.sort();
        assert_eq!(constants, ["LIMIT", "None"]);
        assert_eq!(captured_text(&tree, source, "type"), ["Point"]);
        assert!(
            captured_text(&tree, source, "string")
                .contains(&"\"\"\"Doc\nat column zero\"\"\"".to_string())
        );

        assert_eq!(
            text_object_at(&tree, source, "return label").as_deref(),
            Some("def inner():\n        return label")
        );
        let outer = text_object_at(&tree, source, "label = ").unwrap();
        assert!(outer.starts_with("async def outer"));
        assert!(outer.ends_with("return LIMIT, Point(1), None"));
        assert_eq!(text_object_at(&tree, source, "x = 1"), None);
    }

    #[test]
    fn default_parsers_tolerate_incomplete_code() {
        let runtime = PythonRuntime::new().unwrap();
        for source in [
            "",
            "fn",
            "def",
            "fn broken( {",
            "def broken(:\n    s = 'open",
            "}}} fn a() { \"open",
            "/* open",
            "'''",
            "r#\"",
        ] {
            for language in ["rust", "python"] {
                runtime
                    .parse_syntax(language, source)
                    .unwrap_or_else(|error| panic!("{language} parser failed: {error}"));
            }
        }
    }

    #[test]
    fn invalid_python_parser_ast_ranges_are_reported() {
        let runtime = PythonRuntime::new().unwrap();
        Python::attach(|py| {
            let module = runtime.module.bind(py);
            let source = CString::new(
                "register_language('invalid-test', ('bad',),\n\
                 parser=lambda source: ast_node('root', 0, len(source) + 1))",
            )
            .unwrap();
            py.run(source.as_c_str(), Some(&module.dict()), None)
                .unwrap();
        });

        let error = runtime.parse_syntax("invalid-test", "x").unwrap_err();
        assert!(error.contains("invalid range"));
    }

    #[test]
    fn select_function_command_requests_editor_text_object_selection() {
        let runtime = PythonRuntime::new().unwrap();
        runtime.execute("select-function").unwrap();

        assert_eq!(
            PythonRuntime::take_actions(),
            vec![PythonAction::SelectFunction]
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
            "select-function",
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
