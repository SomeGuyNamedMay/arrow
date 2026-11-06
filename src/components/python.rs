use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::CString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PythonAction {
    OpenBuffer(String),
}

thread_local! {
    static PYTHON_ACTIONS: RefCell<Vec<PythonAction>> = const { RefCell::new(Vec::new()) };
}

const ARROW_PYTHON_API: &str = r#"
commands = {}
hooks = {}

def command(name=None):
    def register(function):
        command_name = name or function.__name__.replace("_", "-")
        commands[command_name] = function
        return function
    return register

def hook(name):
    def register(function):
        hooks.setdefault(name, []).append(function)
        return function
    return register
"#;
const DEFAULT_COMMANDS: &str = include_str!("../../python/default_commands.py");

#[pyfunction]
fn open_file(path: &str) -> PyResult<String> {
    let contents = std::fs::read_to_string(path).map_err(|error| {
        PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("Could not open '{path}': {error}"))
    })?;
    PYTHON_ACTIONS.with(|actions| {
        actions
            .borrow_mut()
            .push(PythonAction::OpenBuffer(contents.clone()));
    });
    Ok(contents)
}

#[pyfunction]
fn write_file(path: &str, contents: &str) -> PyResult<()> {
    if let Some(parent) = std::path::Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            PyErr::new::<pyo3::exceptions::PyOSError, _>(format!(
                "Could not create directory '{}': {error}",
                parent.display()
            ))
        })?;
    }
    std::fs::write(path, contents).map_err(|error| {
        PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("Could not write '{path}': {error}"))
    })
}

pub struct PythonRuntime {
    module: Py<PyModule>,
    commands: BTreeMap<String, String>,
}

impl PythonRuntime {
    pub fn take_actions() -> Vec<PythonAction> {
        PYTHON_ACTIONS.with(|actions| std::mem::take(&mut *actions.borrow_mut()))
    }

    pub fn new() -> Result<Self, String> {
        Python::attach(|py| {
            let code = CString::new(ARROW_PYTHON_API).unwrap();
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
            module
                .add_function(wrap_pyfunction!(open_file, py).unwrap())
                .map_err(|error| format!("Failed to register open_file: {error}"))?;
            module
                .add_function(wrap_pyfunction!(write_file, py).unwrap())
                .map_err(|error| format!("Failed to register write_file: {error}"))?;

            let default_commands = CString::new(DEFAULT_COMMANDS).unwrap();
            py.run(default_commands.as_c_str(), Some(&module.dict()), None)
                .map_err(|error| format!("Failed to load default Python commands: {error}"))?;

            let init_path = config_directory().join("init.py");
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

    pub fn command_names(&self) -> Vec<String> {
        self.commands.keys().cloned().collect()
    }

    pub fn has_command(&self, name: &str) -> bool {
        self.commands.contains_key(name)
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

pub fn config_directory() -> std::path::PathBuf {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return std::path::PathBuf::from(config_home).join("arrow");
    }

    if let Some(app_data) = std::env::var_os("APPDATA") {
        return std::path::PathBuf::from(app_data).join("Arrow");
    }

    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".config")
        .join("arrow")
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
