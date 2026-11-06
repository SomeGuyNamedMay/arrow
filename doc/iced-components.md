# Iced Components

Arrow is built with Iced and organizes its UI into stateful components under
`src/components/`. Each component owns its state and input handling, while the
application shell in `src/main.rs` coordinates subscriptions and composition.

## Application shell (`src/main.rs`)

The application shell implements the Iced model-view-update loop:

- `Arrow` is the top-level state. It contains the `TextEditor` buffer and the
  `Vertico` command menu.
- `AppMessage` is the top-level event type. `Editor` wraps messages produced by
  the text editor; `Key` carries keyboard input from the global subscription.
- `update` mutates the application state. Tab toggles Vertico. When Vertico is
  visible, it handles keyboard input first; otherwise the editor receives the
  key.
- `subscription` listens to Iced keyboard events and forwards them as
  `AppMessage::Key`.
- `view` composes the editor, command menu, and mode/status text into a column.
  Component views are mapped to `AppMessage` with `Element::map`.

This hierarchy keeps global routing in the application shell and component
behavior in the components themselves.

## Text editor (`src/components/text_editor.rs`)

### Public state

`TextEditor` is the editor's state model. Its important fields are:

- `buffer`: the UTF-8 document stored as a `String`.
- `cursor`: the primary cursor index.
- `cursors`: the character positions of every active cursor.
- `anchor` and `anchors`: selection anchors for the primary and additional
  cursors.
- `mode`: normal or insert mode.
- `yank`: the text used by paste operations.

Character indexes, rather than byte indexes, are used throughout the editing
logic. Byte indexes are calculated only when operations need to modify Rust
string storage.

### Messages and updates

`Message::Key` carries an Iced keyboard key and optional text. Normal mode maps
navigation and editing commands to component methods; insert mode inserts text,
newlines, and handles backspace and Escape.

Normal-mode commands include:

- `h`, `j`, `k`, and `l` or arrow keys for movement.
- `w` and `b` for word movement.
- `x` for selecting the current line.
- `i` and `a` for entering insert mode before or after the cursor.
- `c` for adding a cursor on the next line.
- `d` for deleting the selection or character.
- `p` for paste.

Operations that mutate text or move cursor positions iterate over all active
cursors. Text edits process positions from right to left so that insertions and
deletions before later cursors do not invalidate their indexes.

### Position and selection helpers

The buffer is indexed as Unicode characters. Helpers convert those logical
indexes to two-dimensional positions and back again:

- `position_at` finds the row and column for one character index.
- `move_*` methods derive a new position for each cursor.
- `selected_ranges` produces selection ranges for rendering and editing.
- `byte_index` converts a character index to a Rust string byte offset.

Multiple selection anchors are kept in `anchors`, with legacy support for the
primary `anchor` field where a single selection is still used.

### Canvas rendering

`TextEditor::view` creates an Iced `Canvas` and stores a reference to the editor
state in `EditorCanvas`.

`EditorCanvas::draw` owns presentation:

- Draws the dark editor background.
- Splits the buffer into lines and draws each line with a monospace font.
- Draws selection rectangles for every `selected_ranges` entry.
- Measures text prefixes with `character_width` to align cursors and selections
  precisely at monospace character boundaries.
- Draws a cursor block for every position in `cursors`.
- Uses insert-mode and normal-mode colors to distinguish the editor mode.

Canvas rendering is intentionally derived from editor state rather than
maintaining separate display state. Coordinates use `TEXT_X`, `TEXT_Y`, and
`LINE_HEIGHT`.

### Tests

The component's unit tests cover text insertion, cursor positioning, line
selection, cursor creation, and multi-cursor insertion and deletion.

## Command menu (`src/components/vertico.rs`)

`Vertico` is a stateful command-palette component similar to a minibuffer.

### Public state

- `visible` controls whether the menu is rendered.
- `query` contains text entered by the user.
- `selected` identifies the filtered candidate currently highlighted.
- `candidates` contains the built-in command names.

The `Default` implementation starts Vertico visible with the built-in candidate
list.

### Input handling

`update_key` consumes keys owned by the menu and returns `true` when handled.
Escape hides the menu, arrow keys move the selection, and backspace removes
from the query. Printable text is appended through the optional text supplied
by Iced.

`toggle` opens and closes the menu and resets query state. The application shell
still handles Tab before delegating the key, which ensures Tab can always show
or hide the menu.

### Filtering and navigation

`filtered` performs case-insensitive subsequence filtering against every
candidate. For example, `sf` matches both `save-buffer` and `switch-buffer`.

`move_selection` wraps navigation and handles an empty result without indexing
an invalid candidate.

### Widget rendering

`Vertico::view` is a conventional widget tree rather than a custom canvas:

- A horizontal rule separates the palette from surrounding UI.
- A monospace prompt row displays `M-x` and the current query.
- A second rule separates the prompt from results.
- A `column` contains up to eight filtered candidates.
- Each candidate row is marked with `>` when selected and receives a highlighted
  container style.

The component returns an empty shrinking container when hidden, allowing the
application layout to stay stable.

### Tests

Vertico tests cover filtering, query editing, wrapped selection movement,
Escape, and toggling state.

## Component registration (`src/components/mod.rs`)

`src/components/mod.rs` exports the component modules:

```rust
pub mod text_editor;
pub mod vertico;
pub mod file_explorer;
```

The application imports types and messages from these modules in
`src/main.rs`, then wires them into the shared application update loop.

## Python scripting (`src/components/python.rs`)

Arrow embeds Python through PyO3 and loads Emacs-style commands and hooks from
`init.py`. On Windows, this file is read from `%APPDATA%\Arrow\init.py`; on
other platforms it is read from `$XDG_CONFIG_HOME/arrow/init.py`, or
`~/.config/arrow/init.py` when `XDG_CONFIG_HOME` is unset.

Commands are registered by decorating functions in Python:

```python
import arrow

@arrow.command()
def greet_user():
    print("Hello from Arrow")

@arrow.command("open-project-notes")
def open_project_notes():
    print("Project notes command")

@arrow.hook("startup")
def startup():
    print("Arrow Python configuration loaded")
```

The decorator name becomes `greet-user` when no explicit name is supplied.
Python commands are added to the Vertico command menu and execute when Enter is
pressed on the selected entry. Startup hooks run once after `init.py` is loaded.
Python exceptions are reported in the application stderr and do not stop Arrow.

## File explorer (`src/components/file_explorer.rs`)

`FileExplorer` is a ranger-style component organized into parent, current, and
preview panes. It is closed by default and is toggled with `e` from normal
editor mode.

### State and filesystem access

The component owns:

- `visible`, which controls whether the explorer is rendered.
- `current_dir`, the directory currently shown in the middle pane.
- `entries`, the sorted directories and files in that directory.
- `selected`, the index of the current entry.

Directories are sorted before files. Both groups are sorted case-insensitively
by file name. Filesystem reads are performed directly through `std::fs`.
Navigator errors such as an unreadable directory do not change the component's
visible state.

### Input and navigation

Arrow keys and `j`/`k` move selection with wraparound. `h` or the left arrow
opens the parent directory; `l`, the right arrow, or Enter opens a selected
directory. Escape closes the explorer.

`update_key` returns whether it handled the key, allowing the application shell
to give explorer input priority while it is visible.

### Layout and previews

The three-pane widget layout presents the parent directory, current directory,
and preview:

- Directory selection previews the selected directory's entries.
- File selection attempts to read and display the first 50 lines of the file.
- A selected unreadable or binary file produces an empty preview rather than
  disrupting the interface.

The component is rendered with the standard Iced widget tree and uses monospace
text and highlighted containers consistent with Vertico.

## Design conventions

- Component state lives in Rust structs and is updated immutably from Iced's
  perspective, even though methods mutate state in place.
- Public component data is kept separate from private implementation data.
- Keyboard routing is explicit: parent messages route keys to child components
  or to the application as a whole.
- Views derive presentation from state, avoiding renderer caches in model state.
- Canvas components are used for positional text rendering; ordinary widgets
  are used when layout can be expressed as rows, columns, containers, and rules.
- Unit tests live beside component implementations and exercise state transitions
  without constructing an Iced window.
