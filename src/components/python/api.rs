use super::{EDITOR_TEXT, PYTHON_ACTIONS, PythonAction};
use crate::components::text_editor::{Dot, SelectionRange};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};

pub(super) const ARROW_PYTHON_API: &str = r#"
commands = {}
hooks = {}
keybindings = {}
syntax_languages = {}

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

def register_language(name, grammar, extensions, highlights_query,
                      function_nodes=(), capture_colors=None):
    """Register a compiled grammar's extensions, queries, and highlight styles.

    Capture colors map query names to (red, green, blue, priority), with color
    channels between 0.0 and 1.0. Grammar identifiers select parsers compiled
    into Arrow; see builtin_highlights_query for available bundled queries.
    """
    syntax_languages[name] = {
        "name": name,
        "grammar": grammar,
        "extensions": tuple(extensions),
        "highlights_query": highlights_query,
        "function_nodes": tuple(function_nodes),
        "capture_colors": dict(capture_colors or {}),
    }

def dots():
    return _editor_dots.copy()

def set_dots(new_dots):
    new_dots = list(new_dots)
    current_dots = dots()
    converted = []
    for index, dot in enumerate(new_dots):
        default_focus = (
            current_dots[index]["is_focused"]
            if index < len(current_dots)
            else index == len(new_dots) - 1
        )
        if isinstance(dot, dict):
            cursor = dot["cursor"]
            selection = dot.get("selection")
            is_focused = dot.get("is_focused", default_focus)
        else:
            cursor, selection, *focus = dot
            is_focused = focus[0] if focus else default_focus
        if selection is not None:
            selection = (selection[0], selection[1])
        converted.append((cursor, selection, is_focused))
    arrow_set_dots(converted)

def focused_dots():
    return [dot for dot in dots() if dot["is_focused"]]

def set_dot_focused(index, is_focused=True):
    current_dots = dots()
    current_dots[index]["is_focused"] = bool(is_focused)
    set_dots(current_dots)

def focus_all_dots():
    current_dots = dots()
    for dot in current_dots:
        dot["is_focused"] = True
    set_dots(current_dots)

def unfocus_all_dots():
    current_dots = dots()
    for dot in current_dots:
        dot["is_focused"] = False
    set_dots(current_dots)

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
        actions.borrow_mut().push(PythonAction::OpenBuffer {
            contents: contents.clone(),
            path: path.to_string(),
        });
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
fn builtin_highlights_query(grammar: &str) -> PyResult<&'static str> {
    match grammar {
        "rust" => Ok(tree_sitter_rust::HIGHLIGHTS_QUERY),
        "python" => Ok(tree_sitter_python::HIGHLIGHTS_QUERY),
        _ => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "No built-in grammar named '{grammar}'"
        ))),
    }
}

#[pyfunction]
fn arrow_set_dots(dots: Vec<(usize, Option<(usize, usize)>, bool)>) {
    let dots = dots
        .into_iter()
        .map(|(cursor, selection, is_focused)| Dot {
            cursor,
            selection: selection.map(|(start, end)| SelectionRange { start, end }),
            is_focused,
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
        item.set_item("is_focused", dot.is_focused)?;
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
queue_action!(select_function, PythonAction::SelectFunction);

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
    module
        .add_function(wrap_pyfunction!(builtin_highlights_query, py).unwrap())
        .map_err(|error| format!("Failed to register builtin_highlights_query: {error}"))?;
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
        wrap_pyfunction!(select_function, py).unwrap(),
    ] {
        module
            .add_function(function)
            .map_err(|error| format!("Failed to register Arrow Python action: {error}"))?;
    }
    Ok(())
}
