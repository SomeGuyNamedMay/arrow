use super::{EDITOR_TEXT, PYTHON_ACTIONS, PythonAction};
use crate::components::text_editor::{Dot, SelectionRange};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};

pub(super) const ARROW_PYTHON_API: &str = r#"
commands = {}
hooks = {}
keybindings = {}

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

def bind_key(key, command, mode="normal"):
    keybindings.setdefault(mode, {})[key] = command
def keybinding(key, command, mode="normal"):
    bind_key(key, command, mode)

def dots():
    return _editor_dots.copy()

def set_dots(new_dots):
    converted = []
    for dot in new_dots:
        if isinstance(dot, dict):
            cursor = dot["cursor"]
            selection = dot.get("selection")
        else:
            cursor, selection = dot
        if selection is not None:
            selection = (selection[0], selection[1])
        converted.append((cursor, selection))
    arrow_set_dots(converted)

def show_vertico(title, items, action, prompt=""):
    normalized = []
    for item in items:
        if isinstance(item, dict):
            label = item.get("label", item.get("value", ""))
            arguments = item.get("arguments", [item.get("value", "")])
            normalized.append((label, [str(argument) for argument in arguments]))
        else:
            label, *arguments = item
            normalized.append((label, [str(argument) for argument in arguments]))
    arrow_show_vertico(title, normalized, action, prompt)

def run_command(name):
    _run_command(name)

def text():
    return editor_text()

def replace(start, end, replacement):
    replace_ranges([(start, end, replacement)])

def insert(position, replacement):
    replace(position, position, replacement)

def delete(start, end):
    replace(start, end, "")
"#;

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

#[pyfunction]
fn enter_insert_mode() {
    PYTHON_ACTIONS.with(|actions| {
        actions.borrow_mut().push(PythonAction::EnterInsertMode);
    });
}

#[pyfunction]
fn leave_insert_mode() {
    PYTHON_ACTIONS.with(|actions| {
        actions.borrow_mut().push(PythonAction::LeaveInsertMode);
    });
}

#[pyfunction]
fn toggle_file_explorer() {
    PYTHON_ACTIONS.with(|actions| {
        actions.borrow_mut().push(PythonAction::ToggleFileExplorer);
    });
}

#[pyfunction(name = "arrow_show_vertico")]
fn arrow_show_vertico(title: &str, items: Vec<(String, Vec<String>)>, action: &str, prompt: &str) {
    PYTHON_ACTIONS.with(|actions| {
        actions.borrow_mut().push(PythonAction::ShowVertico {
            title: title.to_string(),
            prompt: prompt.to_string(),
            action: action.to_string(),
            items,
        });
    });
}

#[pyfunction(name = "_run_command")]
fn run_command(name: &str) {
    PYTHON_ACTIONS.with(|actions| {
        actions
            .borrow_mut()
            .push(PythonAction::ExecuteCommand(name.to_string()));
    });
}

#[pyfunction]
fn arrow_set_dots(dots: Vec<(usize, Option<(usize, usize)>)>) {
    let dots = dots
        .into_iter()
        .map(|(cursor, selection)| Dot {
            cursor,
            selection: selection.map(|(start, end)| SelectionRange { start, end }),
        })
        .collect();
    PYTHON_ACTIONS.with(|actions| {
        actions.borrow_mut().push(PythonAction::ReplaceDots(dots));
    });
}

#[pyfunction]
fn editor_text() -> String {
    EDITOR_TEXT.with(|text| text.borrow().clone())
}

#[pyfunction]
fn replace_ranges(ranges: Vec<(usize, usize, String)>) {
    PYTHON_ACTIONS.with(|actions| {
        actions
            .borrow_mut()
            .push(PythonAction::ReplaceRanges(ranges));
    });
}

pub(super) fn publish_editor_dots(module: &Bound<'_, PyModule>, dots: Vec<Dot>) -> PyResult<()> {
    let py_dots = PyList::empty(module.py());
    for dot in dots {
        let item = PyDict::new(module.py());
        item.set_item("cursor", dot.cursor)?;
        item.set_item(
            "selection",
            dot.selection
                .map(|selection| (selection.start, selection.end)),
        )?;
        py_dots.append(item)?;
    }
    module.setattr("_editor_dots", py_dots)
}

macro_rules! queue_action {
    ($name:ident, $action:path) => {
        #[pyfunction]
        fn $name() {
            PYTHON_ACTIONS.with(|actions| actions.borrow_mut().push($action));
        }
    };
}

queue_action!(close_file_explorer, PythonAction::CloseFileExplorer);
queue_action!(explorer_selection_up, PythonAction::ExplorerSelectionUp);
queue_action!(explorer_selection_down, PythonAction::ExplorerSelectionDown);
queue_action!(
    explorer_parent_directory,
    PythonAction::ExplorerParentDirectory
);
queue_action!(explorer_open_selected, PythonAction::ExplorerOpenSelected);
queue_action!(close_vertico, PythonAction::CloseVertico);
queue_action!(vertico_selection_up, PythonAction::VerticoSelectionUp);
queue_action!(vertico_selection_down, PythonAction::VerticoSelectionDown);
queue_action!(vertico_backspace, PythonAction::VerticoBackspace);
queue_action!(
    explorer_activate_selected,
    PythonAction::ExplorerActivateSelected
);
queue_action!(execute_vertico, PythonAction::ExecuteVertico);

pub(super) fn register(py: Python<'_>, module: &Bound<'_, PyModule>) -> Result<(), String> {
    module
        .add_function(wrap_pyfunction!(open_file, py).unwrap())
        .map_err(|error| format!("Failed to register open_file: {error}"))?;
    module
        .add_function(wrap_pyfunction!(write_file, py).unwrap())
        .map_err(|error| format!("Failed to register write_file: {error}"))?;
    module
        .add_function(wrap_pyfunction!(enter_insert_mode, py).unwrap())
        .map_err(|error| format!("Failed to register enter_insert_mode: {error}"))?;
    module
        .add_function(wrap_pyfunction!(leave_insert_mode, py).unwrap())
        .map_err(|error| format!("Failed to register leave_insert_mode: {error}"))?;
    module
        .add_function(wrap_pyfunction!(toggle_file_explorer, py).unwrap())
        .map_err(|error| format!("Failed to register toggle_file_explorer: {error}"))?;
    module
        .add_function(wrap_pyfunction!(arrow_set_dots, py).unwrap())
        .map_err(|error| format!("Failed to register set_dots: {error}"))?;
    module
        .add_function(wrap_pyfunction!(arrow_show_vertico, py).unwrap())
        .map_err(|error| format!("Failed to register show_vertico: {error}"))?;
    module
        .add_function(wrap_pyfunction!(run_command, py).unwrap())
        .map_err(|error| format!("Failed to register execute_command: {error}"))?;
    module
        .add_function(wrap_pyfunction!(editor_text, py).unwrap())
        .map_err(|error| format!("Failed to register editor_text: {error}"))?;
    module
        .add_function(wrap_pyfunction!(replace_ranges, py).unwrap())
        .map_err(|error| format!("Failed to register replace_ranges: {error}"))?;
    module.setattr("_editor_dots", PyList::empty(py)).unwrap();
    for function in [
        wrap_pyfunction!(close_file_explorer, py).unwrap(),
        wrap_pyfunction!(explorer_selection_up, py).unwrap(),
        wrap_pyfunction!(explorer_selection_down, py).unwrap(),
        wrap_pyfunction!(explorer_parent_directory, py).unwrap(),
        wrap_pyfunction!(explorer_open_selected, py).unwrap(),
        wrap_pyfunction!(close_vertico, py).unwrap(),
        wrap_pyfunction!(vertico_selection_up, py).unwrap(),
        wrap_pyfunction!(vertico_selection_down, py).unwrap(),
        wrap_pyfunction!(vertico_backspace, py).unwrap(),
        wrap_pyfunction!(explorer_activate_selected, py).unwrap(),
        wrap_pyfunction!(execute_vertico, py).unwrap(),
    ] {
        module
            .add_function(function)
            .map_err(|error| format!("Failed to register Arrow Python action: {error}"))?;
    }
    Ok(())
}
