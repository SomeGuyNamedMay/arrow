# Arrow

A Kakoune-style modal text editor in Rust (iced GUI). The host stores text and
draws it; **all editing behaviour comes from WebAssembly component plugins**
(wasmtime), including the built-in modes, motions, undo and key bindings.

`README.md` is the user-facing guide. It covers the key bindings, the config
folder (`languages.toml`, grammars, queries) and what plugins can do. Update it
when you change a default binding, the config format or the plugin API.

## Build and test

- `cargo test` runs everything that matters: unit tests plus end-to-end
  session tests that load the real bundled plugins and type keys.
- `cargo build` / `cargo run` start the editor.
- Requirements:
  - the `wasm32-wasip2` target (`rustup target add wasm32-wasip2`), because
    `build.rs` builds the plugins in `plugins/` with it.
  - `cmake` on PATH: tree-sitter's `wasm` feature (via `wasmtime-c-api`) runs
    it to generate headers. On this machine it may only exist inside Visual
    Studio Build Tools (`...\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin`).
- `build.rs` rebuilds plugins automatically when `wit/` or `plugins/*/src`
  change, into `target/plugins`, and the host embeds them with `include_bytes!`.
- `LNK4217` linker warnings on Windows come from tree-sitter's C code and are
  harmless.
- The code isn't rustfmt-formatted. Don't run `cargo fmt` over the whole
  workspace as part of an unrelated change.

## Layout

```
wit/arrow.wit            the plugin API (the contract between host and plugins)
build.rs                 builds plugins/* to wasm and copies them to OUT_DIR
src/
  main.rs                module overview, main()
  app.rs                 iced window: key inbox, Frame snapshot, async dispatch
  session/mod.rs         Session: key → command → events → follow-ups → syntax
  session/write.rs       saving a buffer to its file (the `write` app command)
  terminal/              scratchpad terminal: mod.rs (pty, shell, screen,
                         transcript), panel.rs (show/hide/focus, owned by App),
                         keys.rs (keys → xterm bytes), view.rs, config.rs
  session/tests.rs       end-to-end tests through the bundled plugins
  config.rs              user config dir (%APPDATA%\arrow, ~/.config/arrow)
  plugins/mod.rs         loading/calling plugins, Core (state lent to plugins)
  plugins/host_api.rs    host implementations of the WIT interfaces
  plugins/registry.rs    modes, commands, key bindings, subscriptions
  plugins/buffers.rs     multiple buffers on Core (open/switch/close/list)
  plugins/snapshots.rs   undo snapshots on Core (rope clones, by id)
  plugins/compile_cache.rs  compiled plugins saved between runs
  plugins/processes.rs   programs plugins run (`process` interface),
                         `Programs` permissions, `plugins.toml`
  syntax/                tree-sitter: config.rs (languages.toml, theme),
                         highlight.rs (colours for visible text), mod.rs (parsing)
  components/            widgets: text_editor/ (Editor storage + view),
                         menu.rs, tree_menu.rs, keys.rs (Kakoune key notation)
plugins/
  core/                  modes, motions, editing, dot shifting, merging, undo
  text-tools/            sort-lines, duplicate-line, count-words, event hooks
  files/                 ranger-style file manager, buffer list and switching
  git/                   `git status` menu: the example user of the process API
tests/fixtures/          demo tree-sitter grammar + config used by syntax tests
```

## Design rules

- **The host only stores and draws.** `Editor::replace` doesn't move dots,
  merge them or keep the final newline. The core plugin does that (its `edit`
  shifts every dot). The host's one rule is that dots stay inside the text and
  there is always at least one. Behaviour belongs in a plugin, not in the host.
  - **Exception: mechanical helpers for speed.** These are things that would
    be slow across the wasm boundary. Plugins still decide when to use them:
    - `edit` replaces text and shifts dots by core's rule
      (`Editor::replace_shifting`);
    - `line-start`, `line-end` and `line-index` use the rope's line index;
    - undo snapshots (`src/plugins/snapshots.rs`) are rope clones with a
      revision counter.
  - **Keep the core plugin off the slow paths.** Never copy the whole buffer
    into it (`slice(0, len)`) and never loop over `char-at`. Use the helpers,
    or core's `char_at` window, which must be cleared (`forget_window`) after
    any change to the text.
- **Positions are character indices**, not bytes. A dot is a selection
  `min(anchor, cursor)..=max(anchor, cursor)`. The last dot is the primary one.
  Tree-sitter uses bytes, so `syntax::input_edit` converts between the two.
- **Text is a `ropey::Rope`** built with `unicode_lines` off, so only `\n` ends
  a line. Cloning is cheap, which the window's `Frame` snapshot relies on.
- **Buffers end with `\n`.** The core plugin keeps this, and the files plugin
  adds one when it loads a file.
- **Async model:** `App` moves the `Session` into an iced `Task` for each batch
  of key presses and gets it back in `Message::Done`. Keys pressed in the
  meantime wait in `App::inbox`, and the view draws `Frame`. Plugin calls use
  wasmtime `call_async`: they yield every epoch tick and are interrupted after
  `CALL_TICKS` (init: `INIT_TICKS`). A plugin that traps or times out is
  disabled for the rest of the run.
- **During a plugin call, `Core` is moved into that plugin's store**
  (`PluginHost::call`). Plugins can't call each other directly: `run-command`
  and menu requests are queued, and `Session::follow_up` runs them after the
  call returns (capped at `MAX_FOLLOW_UPS`).
- **Buffers:** `Core::editor` is the current buffer, and the others live in
  `Core::hidden`. The editor API always works on the current buffer. The mode
  is shared by all buffers. Ids are never reused. Each buffer has its own core
  plugin undo history (`follow_buffer`) and its own syntax tree (`Syntax`
  keyed by `BufferId`). The yank register is shared.
- **Syntax:** user grammars are `.wasm` files in `<config>/grammars/`, chosen
  by `<config>/languages.toml` from the buffer name's extension. Queries are
  in `<config>/queries/<lang>/highlights.scm`. Each batch's edits are reparsed
  incrementally in `Session::update_syntax`; only visible lines are
  highlighted. tree-sitter brings its own wasmtime (48), separate from the
  plugin engine (50).
- **Saving is the host's job.** `write` is an app command, next to `quit`, in
  `session/mod.rs`. Plugins can bind it or `run-command` it, but they can't
  write files themselves. A buffer's name is its path relative to the working
  directory. Names in `*stars*` aren't files. Paths with `..` or absolute
  paths are refused. Writes go to a temporary file, which is then renamed over
  the target.
- **Unsaved changes:** `Editor::is_modified` compares the text with the copy
  kept at open or save (`mark_saved`), with a revision counter as the fast
  path. `Session::start` marks the startup text saved, so core's final-newline
  edit doesn't count. The `[+]` in the status bar, `buffer-info.modified`, and
  the files plugin's refusal to close a modified buffer (`discard-buffer`
  forces it) all come from this.
- **The terminal belongs to `App`, not `Session`.**
  - **Why:** it keeps running and drawing while a task holds the session.
  - **Shell:** `portable-pty` (ConPTY on Windows) runs it, and `vt100` keeps the
    screen. Two threads report output and exit as `terminal::Event`s, which
    `Task::run` turns into `Message::Terminal(generation, _)`. Events from a
    replaced shell (an older generation) are ignored.
  - **Commands:** `terminal` and `terminal-toggle` are app commands. They only
    set `Session::terminal_request`. `Session::press_all` stops the batch
    there and returns the remaining keys, and `App` gives them to the terminal.
  - **Keys:** while the terminal has focus, `App::pump` sends keys straight to
    `Panel::press`. Ctrl-t (back to the editor) and Alt-t (hide) are the only
    keys the shell never sees.
  - **Cursor queries:** ConPTY asks where the cursor is while PowerShell
    starts, and waits for the answer. `Replies` in `terminal/mod.rs` answers.
    Don't remove it.
  - **Transcript:** raw output is appended to `<config>/state/terminal.log`.
    On the next start its tail (256 KiB, cut at a line) is replayed before the
    new shell's output.
- **Startup is plugin compilation, so it is cached.**
  - **The cache:** compiled plugins (bundled and user) are saved in
    `<cache>/plugins/<hash>.cwasm`, where `<cache>` is `config::user_cache_dir`
    (`%LOCALAPPDATA%\arrow\cache`). The hash covers the plugin's bytes and
    wasmtime's engine compatibility hash, so any change just misses.
  - **Loading:** a stale or damaged entry is compiled again. Tests pass no
    cache (`Session::start(_, false)`), so they never touch the user's folder.
  - **Debug builds:** `Cargo.toml` sets `opt-level = 3` for the Cranelift and
    wasmtime compiler crates, because unoptimised Cranelift made debug startup
    take seconds.
  - **Numbers:** startup in debug was 2.7 s; it is now 0.3 s on a first
    launch and under 20 ms afterwards.
  - **Changing engine settings** in `plugins::engine` is fine: the
    compatibility hash changes, so old entries miss.
  - **Shown while loading:** "loading plugins…" in the status bar until the
    session arrives. Keys typed meanwhile wait in `App::inbox`.
- **Plugin sandboxing:**
  - **Default:** plugins get stdout and stderr only.
  - **`Access` widens it:**
    - `read_working_dir` gives read-only access to the working directory (the
      files plugin).
    - `programs` lists what a plugin may run through the `process` interface.
  - **Bundled plugins** get theirs from `BUNDLED` in `src/plugins/mod.rs`. The
    git plugin may run `git`.
  - **User plugins** (`<config>/plugins/*.wasm`) never read files, and run only
    what `<config>/plugins.toml` lists for them. A bare name there allows only
    that bare name, resolved on the PATH; a path allows only that exact path.
- **Programs plugins run** (`src/plugins/processes.rs`):
  - **Threads:** each program has stdout and stderr reader threads, a
    supervisor that sends `Exited` after both readers finish, and a stdin
    writer thread, so `write` never blocks.
  - **Delivery:** events go to `Session::process_events`. `App` takes that
    receiver once, streams it as `Message::Process`, keeps a batch in
    `App::outputs`, and hands it to `Session::deliver` with the next batch
    (even while the terminal has the keys).
  - **`deliver`** sends each event only to the owning plugin, with no
    subscription needed. It wraps each in `observe`/`emit_changes`, so the
    owner's edits cause events, as a command's would.
  - **Cleanup:** an exited program is removed from the registry. Dropping a
    `Process` kills it. A plugin disabled after an error has its programs
    stopped.
  - **Limits:** at most 16 programs per plugin. On Windows they start with
    `CREATE_NO_WINDOW`, so no console window flashes up.

## Common changes

**Adding or changing a host function:**
1. Edit `wit/arrow.wit`. Bump the `package arrow:plugin@x.y.z` version when the
   change breaks existing plugins.
2. Implement it in `src/plugins/host_api.rs`, with state on `Core` or
   `Registry` (registry fields are private; add a method).
3. Bundled plugins pick up the new bindings on the next build.

**Adding a bundled plugin:** create `plugins/<name>/` as a `cdylib` crate using
`wit-bindgen` with `path: "../../wit"`. Add it to the workspace `members` in
`Cargo.toml`, to `BUNDLED` in `build.rs`, and to `BUNDLED` in
`src/plugins/mod.rs` as `(name, bytes, reads working dir, programs it may
run)`. Load order matters: core comes first. A new event variant in the WIT
breaks any plugin whose `on_event` matches every case, so give those a `_` arm.

**Writing tests:**
- Session tests (`src/session/tests.rs`): `app(text)` starts a session with the
  bundled plugins. `type_keys(&mut app, "ione<esc>")` takes Kakoune notation.
  `run(&mut app, "open-file", "Cargo.toml")` runs a command with everything
  that follows it.
- Tests run from the repo root, so the files plugin sees the real tree
  (`src/main.rs`, `Cargo.toml`, `tests/fixtures/...`).
- Tests that write files use `test_dir(name)`, which gives an empty folder
  under `target/arrow-test-writes/`. Never write to tracked files in a test.
- Syntax tests use the demo grammar in `tests/fixtures/syntax`. To rebuild it,
  use the command in `tests/fixtures/grammar-demo/grammar.js`.

## Gotchas

- When editing files with a script (Python or a heredoc in this shell), a
  `\n` inside a replacement string has turned into a real newline and broken
  Rust string literals. Use the Edit tool for any edit containing `\n`.
- `Core` has private fields, so build it with `Core::new(editor, app_commands)`
  rather than struct-update syntax.
- The session test that opens `src/main.rs` checks the file's first line
  (`//! Arrow`). Changing that line breaks the test.
