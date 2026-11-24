# Arrow

A modal text editor in the style of [Kakoune](https://kakoune.org), written in
Rust. The editor itself only stores text and draws it. **Everything it does
(modes, motions, editing, undo, key bindings, the file manager) comes from
plugins**, which are sandboxed WebAssembly components. A plugin you write
uses the same API as the built-in ones and can replace any of them.

Arrow is early work: it edits text, browses, opens and saves files, keeps
several buffers and highlights syntax.

## Building

You need:

- Rust (2024 edition) and the WebAssembly target the plugins are built for:
  ```sh
  rustup target add wasm32-wasip2
  ```
- `cmake` on your PATH. tree-sitter's WebAssembly support uses it while
  building. On Windows it comes with Visual Studio Build Tools, or install it
  with `winget install Kitware.CMake`.

Then:

```sh
cargo run      # build the bundled plugins and start the editor
cargo test     # unit tests, and end-to-end tests through the real plugins
```

The bundled plugins in `plugins/` are built automatically by `build.rs` and
embedded in the executable.

## Using it

Editing happens at **dots**: cursors that are also selections. You select
first, then act, and every command acts at every dot at once.

| Keys | What they do |
|---|---|
| `h` `j` `k` `l`, arrows | move (uppercase, or Shift+arrow, extends the dot instead) |
| `w` `e` `b` | select the next word, to the word's end, the previous word (`W` `E` `B` extend) |
| `x`, `%` | select the line, select everything |
| `gh` `gl` `gg` `ge` | go to line start, line end, buffer start, buffer end (`G…` extends) |
| `c`, `,` | add a dot on the line below the newest one, remove the oldest dot |
| `i` `a` `I` `A` `o` `O` | insert before, after, at line start, at line end, on a new line below or above |
| `Esc` | back to normal mode |
| `d` `s` `y` `p` `P` `R` | delete, change, yank, paste after, paste before, replace with yank |
| `r`, `~`, `` ` `` | replace characters, uppercase, lowercase |
| `>` `<` | indent, dedent |
| `u` `U` | undo, redo |
| `:` or `Alt-x` | command menu: type to filter, Enter to run |
| `?` | list every key binding and what it does |
| `Space f` | file manager (ranger style; `h` `j` `k` `l` to move, Enter to open) |
| `Space b` | switch between open buffers |
| `Ctrl-s` or `:write` | save the buffer to its file |
| `Space k` | close the current buffer (refused if it has unsaved changes; `:discard-buffer` closes it anyway) |
| `Space g` | list the files git reports as changed, and open one |
| `Ctrl-t` | open the terminal and type into it (in the terminal: back to the editor) |
| `Alt-t` | show or hide the terminal |
| `Alt-s`, `Space d`, `Space c` | sort lines, duplicate line, count words (from the text-tools plugin) |

Opening a file gives it a buffer of its own. Opening it again switches back to
that buffer, unsaved edits included. Each buffer has its own undo history,
and the yank register is shared, so you can copy between files.

A buffer with unsaved changes shows `[+]` after its name in the status bar,
and is marked "modified" in the buffer list. Undoing back to the saved text
clears the mark. Saving writes to a temporary file that then replaces the
original, so a failed save never leaves half a file. Only files inside the
folder Arrow was started in can be written.

### The terminal

`Ctrl-t` opens a terminal under the status bar, running your shell in the
folder Arrow was started in. It's a real terminal, so prompts, colours, line
editing and full-screen programs work. While it has the keys, everything goes
to the shell except two: `Ctrl-t` goes back to the editor with the terminal
still showing, and `Alt-t` hides it. `Shift-PageUp` and `Shift-PageDown`
scroll back through its history.

Hiding the terminal doesn't stop the shell: its output, directory and history
are all there when you bring it back. Its output is also saved, so when you
next start Arrow the terminal opens with the previous session's history above
a fresh shell. If the shell exits, any key starts a new one.

## Configuration

Arrow reads your setup from `%APPDATA%\arrow` on Windows, or
`$XDG_CONFIG_HOME/arrow` (usually `~/.config/arrow`) elsewhere:

```
arrow/
  plugins/*.wasm                    your plugins, loaded at startup
  languages.toml                    which grammar to use for which files
  grammars/<grammar>.wasm           tree-sitter grammars
  queries/<language>/highlights.scm highlight queries
  terminal.toml                     the terminal's shell and height
  plugins.toml                      which programs each of your plugins may run
  state/terminal.log                the terminal's saved history (kept by Arrow)
```

Compiled plugins are cached separately, in `%LOCALAPPDATA%\arrow\cache` on
Windows or `~/.cache/arrow` elsewhere, so Arrow starts in a few milliseconds
after its first run. Deleting the cache is always safe; it's rebuilt as needed.

`terminal.toml` is optional:

```toml
shell = "pwsh.exe"   # default: powershell.exe on Windows, $SHELL elsewhere
args = ["-NoLogo"]
rows = 12            # height in lines
```

### Syntax highlighting

Highlighting uses [tree-sitter](https://tree-sitter.github.io) grammars
compiled to WebAssembly, so a grammar runs sandboxed and the same file works
on every platform. Build one from a grammar repository with the tree-sitter
CLI, and copy the repository's `queries/highlights.scm` alongside it:

```sh
tree-sitter build --wasm -o rust.wasm
```

Then list the language in `languages.toml`:

```toml
[[language]]
name = "rust"
extensions = ["rs"]
# grammar = "rust"                            (default: the name)
# highlights = "queries/rust/highlights.scm"  (default)

[theme]
keyword = "#c678dd"   # by capture name; "function.method" falls back to "function"
```

The language follows the buffer's file extension, and the tree is updated
incrementally as you edit.

## Writing plugins

A plugin is a WebAssembly component for the `plugin` world in
[`wit/arrow.wit`](wit/arrow.wit). It can:

- read and change the text and the dots, open, switch and close buffers, and
  see which have unsaved changes;
- run any command, including the editor's own `write` (plugins can't write
  files themselves);
- define modes, register commands and bind keys (in Kakoune notation:
  `"gh"`, `"<a-s>"`, `"<space>d"`);
- subscribe to events: startup, buffer and selection changes, mode changes,
  and commands run;
- open menus and tree menus for the user to pick from;
- run programs and talk to them through their standard input and output, if
  the user allows it (see below).

The easiest way to write one is Rust with
[`wit-bindgen`](https://github.com/bytecodealliance/wit-bindgen), as the
bundled plugins do. [`plugins/text-tools`](plugins/text-tools) is a small
example. Build it for `wasm32-wasip2` and put the `.wasm` in your `plugins/`
folder. Any language that can produce a component works too.

Plugins are sandboxed: each has its own memory, and can reach nothing outside
it but stdout and stderr (the bundled file manager also gets read-only access
to the folder Arrow was started in). A plugin call that runs for more than
about half a second is stopped, and a plugin that crashes or times out is
disabled without taking the editor down.

### Running programs

The `process` interface lets a plugin start a program, such as a proof
assistant, a REPL or a compiler, write to its standard input, and receive its
output. `spawn` returns an id straight away. The output then arrives as
`process-output` events, followed by one `process-exited`, sent only to the
plugin that started the program. Writing never blocks the editor.

Because running a program reaches outside the sandbox, a plugin may only start
the programs you allow, in `plugins.toml` in the config folder:

```toml
[plugins.proof]          # the plugin's file name, without .wasm
run = ["coqtop"]         # programs it may start; ["*"] allows any
```

A bare name like `coqtop` allows that program as found on your PATH, and only
when the plugin asks for it by bare name. A path allows exactly that file. A
plugin that is disabled after a crash has its programs stopped.

[`plugins/git`](plugins/git) is a small example: `Space g` runs `git status`
and lists the changed files in a menu to open.

## How it's built

Plugins run asynchronously, off the UI thread, so a slow one never freezes the
window, and keys you press meanwhile are handled in order. Text is stored in a
[rope](https://docs.rs/ropey), so edits stay fast in large files. See
[`CLAUDE.md`](CLAUDE.md) for a map of the source and the design rules.

## License

GPL-3.0. See [`LICENSE`](LICENSE).
