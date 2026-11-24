//! Programs plugins run, and what they say back.
//!
//! A plugin calls `spawn` (WIT `process` interface). The host starts the
//! program with its stdin, stdout and stderr on pipes, and three threads look
//! after it:
//!
//! - **two readers**, one each for stdout and stderr. They send what they read
//!   as [`ProcessEvent::Output`], in chunks, as soon as it arrives;
//! - **a supervisor**, which waits for both readers to reach end-of-file, then
//!   for the program to exit, and sends [`ProcessEvent::Exited`]. So the exit
//!   always comes after the last output.
//!
//! A fourth thread writes to stdin. `write` only queues bytes for it, so a
//! program that isn't reading can't block the plugin, and with it the editor.
//!
//! The events go down a channel to the window ([`crate::app`]), which hands
//! them to the session in batches when it's free. The session delivers each
//! one to the plugin that started the program. Plugins aren't running between
//! calls, so this is how output reaches them at all.
//!
//! Who may run what is the user's call ([`Programs`]): bundled plugins are
//! allowed theirs by the host; others are listed in `plugins.toml`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::futures::channel::mpsc as async_mpsc;

use super::{Event, types};

pub type ProcessId = u32;

/// Most programs one plugin may have running at once.
const MAX_PER_PLUGIN: usize = 16;

/// The programs a plugin may start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Programs {
    /// None at all (the default).
    #[default]
    None,
    /// These, as described on [`Programs::allows`].
    Listed(Vec<String>),
    /// Any program.
    Any,
}

impl Programs {
    /// From a `run = [...]` list: `"*"` anywhere means any program.
    pub fn from_list(list: Vec<String>) -> Self {
        if list.iter().any(|entry| entry == "*") {
            Self::Any
        } else if list.is_empty() {
            Self::None
        } else {
            Self::Listed(list)
        }
    }

    /// Whether `program` may be started. A bare name in the list ("git")
    /// allows that name only as a bare name, looked up on the PATH, so a plugin
    /// can't run some other file that happens to be called `git` by giving its
    /// path. A path in the list allows exactly that path. On Windows, names
    /// match without regard to case or a trailing `.exe`.
    pub fn allows(&self, program: &str) -> bool {
        match self {
            Self::None => false,
            Self::Any => true,
            Self::Listed(list) => {
                let wanted = normalize(program);
                let bare = is_bare(program);
                list.iter()
                    .any(|entry| is_bare(entry) == bare && normalize(entry) == wanted)
            }
        }
    }
}

fn is_bare(program: &str) -> bool {
    !program.contains(['/', '\\'])
}

fn normalize(program: &str) -> String {
    if cfg!(windows) {
        let lower = program.replace('\\', "/").to_lowercase();
        lower.strip_suffix(".exe").map(str::to_string).unwrap_or(lower)
    } else {
        program.to_string()
    }
}

/// What a program's threads report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessEvent {
    Output {
        id: ProcessId,
        stderr: bool,
        data: Vec<u8>,
    },
    Exited {
        id: ProcessId,
        code: Option<i32>,
    },
}

impl ProcessEvent {
    pub fn id(&self) -> ProcessId {
        match self {
            Self::Output { id, .. } | Self::Exited { id, .. } => *id,
        }
    }

    /// As the event plugins receive.
    pub fn to_event(&self) -> Event {
        match self {
            Self::Output { id, stderr, data } => Event::ProcessOutput(types::ProcessOutput {
                id: *id,
                stream: if *stderr {
                    types::OutputStream::Stderr
                } else {
                    types::OutputStream::Stdout
                },
                data: data.clone(),
            }),
            Self::Exited { id, code } => Event::ProcessExited(types::ProcessExit { id: *id, code: *code }),
        }
    }
}

/// Every program plugins are running, by id. Lives on [`super::Core`].
#[derive(Default)]
pub struct Processes {
    running: HashMap<ProcessId, Process>,
    last_id: ProcessId,
    /// Where the threads send events. Set by the session when it starts; a
    /// core without it (a test's, or the stand-in) can't start programs.
    events: Option<async_mpsc::UnboundedSender<ProcessEvent>>,
}

struct Process {
    /// Index of the plugin that started it: the only one that can use it, and
    /// the one its events go to.
    owner: usize,
    /// Bytes for the stdin thread. `None` once stdin is closed.
    stdin: Option<mpsc::Sender<Vec<u8>>>,
    child: Arc<Mutex<Child>>,
}

impl Drop for Process {
    /// A program nobody is listening to any more is stopped. (Once it has
    /// exited, this does nothing.)
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

/// What a plugin asked to run.
pub struct Spawn {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
}

impl Processes {
    /// Start sending events to `events`, and return the receiving end.
    pub fn connect(&mut self) -> async_mpsc::UnboundedReceiver<ProcessEvent> {
        let (sender, receiver) = async_mpsc::unbounded();
        self.events = Some(sender);
        receiver
    }

    /// Start a program for plugin `owner`, if `allowed` lets it.
    pub fn spawn(&mut self, owner: usize, allowed: &Programs, spawn: Spawn) -> Result<ProcessId, String> {
        if !allowed.allows(&spawn.program) {
            return Err(format!(
                "this plugin isn't allowed to run `{}`: allow it in plugins.toml",
                spawn.program
            ));
        }
        if self.running.values().filter(|p| p.owner == owner).count() >= MAX_PER_PLUGIN {
            return Err(format!("this plugin already has {MAX_PER_PLUGIN} programs running"));
        }
        let events = self
            .events
            .clone()
            .ok_or_else(|| "programs can't be started here".to_string())?;

        let mut command = Command::new(&spawn.program);
        command
            .args(&spawn.args)
            .envs(spawn.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &spawn.cwd {
            command.current_dir(cwd);
        }
        hide_console_window(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("can't start `{}`: {e}", spawn.program))?;

        self.last_id = self.last_id.wrapping_add(1);
        let id = self.last_id;
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let stdin = child.stdin.take().expect("stdin is piped");
        let child = Arc::new(Mutex::new(child));

        let readers = [
            thread("process-stdout", read_into(stdout, id, false, events.clone())),
            thread("process-stderr", read_into(stderr, id, true, events.clone())),
        ];
        let supervised = Arc::clone(&child);
        thread("process-wait", move || {
            for reader in readers.into_iter().flatten() {
                let _ = reader.join();
            }
            let code = wait(&supervised);
            let _ = events.unbounded_send(ProcessEvent::Exited { id, code });
        });
        let (sender, queued) = mpsc::channel();
        thread("process-stdin", move || write_from(stdin, queued));

        self.running.insert(
            id,
            Process {
                owner,
                stdin: Some(sender),
                child,
            },
        );
        Ok(id)
    }

    /// Queue bytes for a program's stdin.
    pub fn write(&mut self, owner: usize, id: ProcessId, data: Vec<u8>) -> Result<(), String> {
        let process = self.owned(owner, id)?;
        let stdin = process
            .stdin
            .as_ref()
            .ok_or_else(|| format!("program {id}'s input is closed"))?;
        stdin
            .send(data)
            .map_err(|_| format!("program {id} isn't reading its input any more"))
    }

    /// Close a program's stdin once queued bytes are written.
    pub fn close_stdin(&mut self, owner: usize, id: ProcessId) {
        if let Ok(process) = self.owned(owner, id) {
            // Dropping the sender ends the stdin thread's loop, which then
            // drops the pipe.
            process.stdin = None;
        }
    }

    pub fn kill(&mut self, owner: usize, id: ProcessId) {
        if let Ok(process) = self.owned(owner, id)
            && let Ok(mut child) = process.child.lock()
        {
            let _ = child.kill();
        }
    }

    /// The plugin a program's events go to.
    pub fn owner(&self, id: ProcessId) -> Option<usize> {
        self.running.get(&id).map(|p| p.owner)
    }

    /// Forget a program that has exited.
    pub fn remove(&mut self, id: ProcessId) {
        self.running.remove(&id);
    }

    /// Stop every program a plugin started, e.g. when it has been disabled.
    pub fn stop_all_of(&mut self, owner: usize) {
        self.running.retain(|_, p| p.owner != owner);
    }

    fn owned(&mut self, owner: usize, id: ProcessId) -> Result<&mut Process, String> {
        self.running
            .get_mut(&id)
            .filter(|p| p.owner == owner)
            .ok_or_else(|| format!("no program {id}"))
    }
}

fn read_into(
    mut pipe: impl Read + Send + 'static,
    id: ProcessId,
    stderr: bool,
    events: async_mpsc::UnboundedSender<ProcessEvent>,
) -> impl FnOnce() + Send + 'static {
    move || {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    let event = ProcessEvent::Output {
                        id,
                        stderr,
                        data: buffer[..n].to_vec(),
                    };
                    if events.unbounded_send(event).is_err() {
                        return;
                    }
                }
            }
        }
    }
}

fn write_from(mut stdin: ChildStdin, queued: mpsc::Receiver<Vec<u8>>) {
    for data in queued {
        if stdin.write_all(&data).and_then(|()| stdin.flush()).is_err() {
            return;
        }
    }
}

/// Wait for the program to exit. Polls rather than blocking in `wait`, so the
/// lock stays free for `kill` in the meantime.
fn wait(child: &Mutex<Child>) -> Option<i32> {
    loop {
        match child.lock().map(|mut child| child.try_wait()) {
            Ok(Ok(Some(status))) => return status.code(),
            Ok(Ok(None)) => std::thread::sleep(Duration::from_millis(20)),
            _ => return None,
        }
    }
}

fn thread(name: &str, work: impl FnOnce() + Send + 'static) -> Option<std::thread::JoinHandle<()>> {
    std::thread::Builder::new().name(name.into()).spawn(work).ok()
}

/// On Windows, a console program started from a GUI app opens a console
/// window of its own unless told not to.
fn hide_console_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// Read `plugins.toml` from `dir`: the programs each named plugin may run.
/// A missing file grants nothing.
///
/// ```toml
/// [plugins.proof]
/// run = ["coqtop"]
/// ```
pub fn load_permissions(dir: &Path) -> Result<HashMap<String, Programs>, String> {
    #[derive(serde::Deserialize, Default)]
    #[serde(deny_unknown_fields)]
    struct File {
        #[serde(default)]
        plugins: HashMap<String, Plugin>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Plugin {
        #[serde(default)]
        run: Vec<String>,
    }
    let path = dir.join("plugins.toml");
    let file: File = match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => File::default(),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    Ok(file
        .plugins
        .into_iter()
        .map(|(name, plugin)| (name, Programs::from_list(plugin.run)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn programs_must_be_allowed_by_name_or_exact_path() {
        let allowed = Programs::from_list(vec!["coqtop".into(), "/opt/lean/bin/lean".into()]);
        assert!(allowed.allows("coqtop"));
        assert!(!allowed.allows("/tmp/coqtop"), "a bare name doesn't allow any path");
        assert!(allowed.allows("/opt/lean/bin/lean"));
        assert!(!allowed.allows("lean"));
        assert!(!allowed.allows("sh"));
        if cfg!(windows) {
            assert!(allowed.allows("CoqTop.exe"));
        }
        assert!(!Programs::None.allows("coqtop"));
        assert!(Programs::from_list(vec!["*".into()]).allows("anything"));
        assert_eq!(Programs::from_list(Vec::new()), Programs::None);
    }

    #[test]
    fn permissions_come_from_plugins_toml() {
        let dir = Path::new("target/arrow-test-writes/permissions");
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        assert!(load_permissions(dir).unwrap().is_empty(), "no file grants nothing");
        std::fs::write(
            dir.join("plugins.toml"),
            "[plugins.proof]\nrun = [\"coqtop\"]\n[plugins.shell]\nrun = [\"*\"]\n",
        )
        .unwrap();
        let permissions = load_permissions(dir).unwrap();
        assert!(permissions["proof"].allows("coqtop"));
        assert_eq!(permissions["shell"], Programs::Any);
        std::fs::write(dir.join("plugins.toml"), "[plugins.x]\nrnu = []\n").unwrap();
        assert!(load_permissions(dir).is_err(), "typos are reported");
    }

    /// Collect events until program `id` exits.
    fn run_to_exit(
        events: &mut async_mpsc::UnboundedReceiver<ProcessEvent>,
        id: ProcessId,
    ) -> (String, Option<i32>) {
        let start = Instant::now();
        let mut output = Vec::new();
        while start.elapsed() < Duration::from_secs(20) {
            match events.try_recv() {
                Ok(ProcessEvent::Output { id: from, data, .. }) if from == id => output.extend(data),
                Ok(ProcessEvent::Exited { id: from, code }) if from == id => {
                    return (String::from_utf8_lossy(&output).into_owned(), code);
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        panic!("program {id} didn't exit; output so far: {}", String::from_utf8_lossy(&output));
    }

    /// A shell reading commands from stdin, so the test can talk to it.
    fn shell() -> Spawn {
        let (program, args) = if cfg!(windows) {
            ("cmd", vec!["/Q", "/K", "prompt $G"])
        } else {
            ("sh", vec![])
        };
        Spawn {
            program: program.into(),
            args: args.into_iter().map(String::from).collect(),
            cwd: None,
            env: vec![("ARROW_TEST".into(), "from-env".into())],
        }
    }

    #[test]
    fn programs_take_input_and_report_output_then_exit() {
        let mut processes = Processes::default();
        let mut events = processes.connect();
        let allowed = Programs::Any;
        let id = processes.spawn(1, &allowed, shell()).unwrap();
        let echo = if cfg!(windows) { "echo %ARROW_TEST%\r\nexit 3\r\n" } else { "echo $ARROW_TEST\nexit 3\n" };
        processes.write(1, id, echo.as_bytes().to_vec()).unwrap();
        assert_eq!(processes.owner(id), Some(1));
        let (output, code) = run_to_exit(&mut events, id);
        assert!(output.contains("from-env"), "{output}");
        assert_eq!(code, Some(3));

        // Only the plugin that started a program can use it.
        let other = processes.spawn(1, &allowed, shell()).unwrap();
        assert!(processes.write(2, other, b"x".to_vec()).is_err());
        processes.kill(1, other);
        let (_, code) = run_to_exit(&mut events, other);
        assert_ne!(code, Some(0), "a killed program doesn't report success");
    }

    #[test]
    fn closing_stdin_ends_a_program_that_reads_to_the_end() {
        let mut processes = Processes::default();
        let mut events = processes.connect();
        let (program, args) = if cfg!(windows) { ("findstr", vec!["x"]) } else { ("grep", vec!["x"]) };
        let spawn = Spawn {
            program: program.into(),
            args: args.into_iter().map(String::from).collect(),
            cwd: None,
            env: Vec::new(),
        };
        let id = processes.spawn(7, &Programs::Any, spawn).unwrap();
        processes.write(7, id, b"one\nxtwo\nthree\n".to_vec()).unwrap();
        processes.close_stdin(7, id);
        let (output, code) = run_to_exit(&mut events, id);
        assert_eq!(output.trim(), "xtwo");
        assert_eq!(code, Some(0));
    }

    #[test]
    fn spawning_needs_permission_and_a_real_program() {
        let mut processes = Processes::default();
        let _events = processes.connect();
        let error = processes.spawn(1, &Programs::None, shell()).unwrap_err();
        assert!(error.contains("plugins.toml"), "{error}");
        let missing = Spawn {
            program: "arrow-no-such-program".into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        };
        assert!(processes.spawn(1, &Programs::Any, missing).is_err());
        // Without a session to receive events, nothing can be started.
        assert!(Processes::default().spawn(1, &Programs::Any, shell()).is_err());
    }
}
