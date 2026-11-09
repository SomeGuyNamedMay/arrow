use ropey::Rope;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use super::syntax::CompletionServerConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionCandidate {
    pub(crate) label: String,
    pub(crate) insert_text: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

type PendingRequests = Arc<Mutex<HashMap<String, mpsc::Sender<Result<Value, String>>>>>;

#[derive(Debug)]
pub(crate) struct LspClient {
    child: Mutex<Child>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: PendingRequests,
    next_id: AtomicU64,
    uri: String,
    language_id: String,
    completion_supported: bool,
}

impl LspClient {
    pub(crate) fn start(
        config: CompletionServerConfig,
        path: &Path,
        language_id: String,
    ) -> Result<Self, String> {
        let path = std::fs::canonicalize(path)
            .map_err(|error| format!("Could not resolve '{}': {error}", path.display()))?;
        let uri = url::Url::from_file_path(&path)
            .map_err(|_| format!("Could not convert '{}' to a file URI", path.display()))?
            .to_string();
        let root = find_root(&path, &config.root_markers);
        let mut command = Command::new(&config.command);
        command
            .args(&config.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(root) = root.as_ref() {
            command.current_dir(root);
        }
        let mut child = command.spawn().map_err(|error| {
            format!(
                "Could not start language server '{}': {error}",
                config.command
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Language server stdin was not available".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Language server stdout was not available".to_string())?;
        let pending: PendingRequests = Arc::default();
        let reader_pending = pending.clone();
        let stdin = Arc::new(Mutex::new(stdin));
        let reader_stdin = stdin.clone();
        std::thread::Builder::new()
            .name("arrow-lsp-reader".to_string())
            .spawn(move || read_messages(stdout, reader_pending, reader_stdin))
            .map_err(|error| format!("Could not start language server reader: {error}"))?;

        let mut client = Self {
            child: Mutex::new(child),
            stdin,
            pending,
            next_id: AtomicU64::new(1),
            uri,
            language_id,
            completion_supported: false,
        };
        let root_uri = root
            .as_ref()
            .and_then(|path| url::Url::from_directory_path(path).ok())
            .map(|url| url.to_string());
        let initialize = client.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "capabilities": {
                    "general": {
                        "positionEncodings": ["utf-16"]
                    },
                    "textDocument": {
                        "completion": {
                            "completionItem": {
                                "snippetSupport": false,
                                "documentationFormat": ["plaintext"]
                            }
                        }
                    }
                },
                "clientInfo": {"name": "Arrow", "version": env!("CARGO_PKG_VERSION")}
            }),
        )?;
        let response = initialize
            .recv_timeout(Duration::from_secs(30))
            .map_err(|error| format!("Language server initialize request failed: {error}"))??;
        if let Some(error) = response.get("error") {
            return Err(format!("Language server initialization failed: {error}"));
        }
        let completion_supported = response
            .pointer("/result/capabilities/completionProvider")
            .is_some_and(|provider| !provider.is_null() && provider != &Value::Bool(false));
        client.notify("initialized", json!({}))?;

        client.completion_supported = completion_supported;
        Ok(client)
    }

    pub(crate) fn did_open(&self, text: &str) -> Result<(), String> {
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": self.uri,
                    "languageId": self.language_id,
                    "version": 1,
                    "text": text
                }
            }),
        )
    }

    pub(crate) fn did_change(&self, version: i32, text: &str) -> Result<(), String> {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": {"uri": self.uri, "version": version},
                "contentChanges": [{"text": text}]
            }),
        )
    }

    pub(crate) fn completion(
        &self,
        source: String,
        cursor: usize,
    ) -> Result<mpsc::Receiver<Result<Vec<CompletionCandidate>, String>>, String> {
        if !self.completion_supported {
            return Err("The configured language server does not support completion".to_string());
        }
        let rope = Rope::from_str(&source);
        let cursor = cursor.min(rope.len_chars());
        let line = rope.char_to_line(cursor);
        let line_start = rope.line_to_char(line);
        let line_prefix: String = rope.slice(line_start..cursor).to_string();
        let character = line_prefix.encode_utf16().count();
        let response = self.request(
            "textDocument/completion",
            json!({
                "textDocument": {"uri": self.uri},
                "position": {"line": line, "character": character},
                "context": {"triggerKind": 1}
            }),
        )?;
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("arrow-lsp-completion".to_string())
            .spawn(move || {
                let result = response
                    .recv_timeout(Duration::from_secs(10))
                    .map_err(|error| format!("Completion request failed: {error}"))
                    .and_then(|response| response)
                    .map(|response| parse_completions(&response, &source, cursor));
                let _ = sender.send(result);
            })
            .map_err(|error| format!("Could not start completion response handler: {error}"))?;
        Ok(receiver)
    }

    fn request(
        &self,
        method: &str,
        params: Value,
    ) -> Result<mpsc::Receiver<Result<Value, String>>, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| "Language server request map is poisoned".to_string())?
            .insert(id.to_string(), sender);
        if let Err(error) = write_message(
            &self.stdin,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        ) {
            self.pending
                .lock()
                .map_err(|_| "Language server request map is poisoned".to_string())?
                .remove(&id.to_string());
            return Err(error);
        }
        Ok(receiver)
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        write_message(
            &self.stdin,
            &json!({"jsonrpc": "2.0", "method": method, "params": params}),
        )
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            match child.kill() {
                Ok(()) => {
                    if let Err(error) = child.wait() {
                        eprintln!("Failed to reap language server: {error}");
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
                    if let Err(error) = child.wait() {
                        eprintln!("Failed to reap language server: {error}");
                    }
                }
                Err(error) => eprintln!("Failed to stop language server: {error}"),
            }
        }
    }
}

fn find_root(path: &Path, markers: &[String]) -> Option<PathBuf> {
    let start = if path.is_dir() { path } else { path.parent()? };
    start
        .ancestors()
        .find(|directory| markers.iter().any(|marker| directory.join(marker).exists()))
        .map(Path::to_path_buf)
        .or_else(|| Some(start.to_path_buf()))
}

fn read_messages(stdout: impl Read, pending: PendingRequests, stdin: Arc<Mutex<ChildStdin>>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut content_length = None;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => return,
                Ok(_) if line == "\r\n" || line == "\n" => break,
                Ok(_) => {
                    if let Some((header, value)) = line.split_once(':')
                        && header.eq_ignore_ascii_case("content-length")
                    {
                        content_length = value.trim().parse::<usize>().ok();
                    }
                }
                Err(error) => {
                    eprintln!("Failed reading language server response: {error}");
                    return;
                }
            }
        }
        let Some(content_length) = content_length else {
            eprintln!("Language server response is missing Content-Length");
            return;
        };
        let mut body = vec![0; content_length];
        if let Err(error) = reader.read_exact(&mut body) {
            eprintln!("Failed reading language server response body: {error}");
            return;
        }
        let message: Value = match serde_json::from_slice(&body) {
            Ok(message) => message,
            Err(error) => {
                eprintln!("Invalid JSON-RPC response from language server: {error}");
                continue;
            }
        };
        let Some(id_value) = message.get("id") else {
            continue;
        };
        if message.get("method").is_some() {
            let result = match message["method"].as_str().unwrap_or_default() {
                "workspace/configuration" => {
                    let count = message["params"]["items"].as_array().map_or(0, Vec::len);
                    Value::Array(vec![Value::Null; count])
                }
                "workspace/workspaceFolders" => Value::Array(Vec::new()),
                "client/registerCapability"
                | "client/unregisterCapability"
                | "window/workDoneProgress/create"
                | "window/showMessageRequest" => Value::Null,
                "workspace/applyEdit" => json!({"applied": false}),
                _ => Value::Null,
            };
            if let Err(error) = write_message(
                &stdin,
                &json!({"jsonrpc": "2.0", "id": id_value, "result": result}),
            ) {
                eprintln!("Failed responding to language server request: {error}");
                return;
            }
            continue;
        }
        let id = json_id(id_value);
        let sender = pending
            .lock()
            .ok()
            .and_then(|mut requests| requests.remove(&id));
        if let Some(sender) = sender {
            let response = if message.get("error").is_some() {
                Err(format!(
                    "Language server request error: {}",
                    message["error"]
                ))
            } else {
                Ok(message)
            };
            let _ = sender.send(response);
        }
    }
}

fn json_id(id: &Value) -> String {
    match id {
        Value::String(value) => value.clone(),
        _ => id.to_string(),
    }
}

fn write_message(stdin: &Arc<Mutex<ChildStdin>>, message: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(message)
        .map_err(|error| format!("Could not encode language server request: {error}"))?;
    let mut stdin = stdin
        .lock()
        .map_err(|_| "Language server stdin is poisoned".to_string())?;
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len())
        .and_then(|()| stdin.write_all(&body))
        .and_then(|()| stdin.flush())
        .map_err(|error| format!("Could not write to language server: {error}"))
}

fn parse_completions(response: &Value, source: &str, cursor: usize) -> Vec<CompletionCandidate> {
    let Some(result) = response.get("result") else {
        return Vec::new();
    };
    let items = result
        .as_array()
        .or_else(|| result.get("items").and_then(Value::as_array));
    let Some(items) = items else {
        return Vec::new();
    };
    let fallback_start = completion_prefix_start(source, cursor);
    items
        .iter()
        .filter_map(|item| {
            let label = item.get("label")?.as_str()?.to_string();
            let mut insert_text = item
                .get("insertText")
                .and_then(Value::as_str)
                .unwrap_or(&label)
                .to_string();
            let mut range = None;
            if let Some(text_edit) = item.get("textEdit") {
                if let Some(new_text) = text_edit.get("newText").and_then(Value::as_str) {
                    insert_text = new_text.to_string();
                }
                let selected_range = text_edit.get("replace").or_else(|| text_edit.get("range"));
                if let Some(selected_range) = selected_range {
                    let start = lsp_position_offset(source, selected_range.get("start")?)?;
                    let end = lsp_position_offset(source, selected_range.get("end")?)?;
                    if start <= end {
                        range = Some((start, end));
                    }
                }
            }
            let (start, end) = range.unwrap_or((fallback_start, cursor));
            Some(CompletionCandidate {
                label,
                insert_text,
                start,
                end,
            })
        })
        .collect()
}

fn completion_prefix_start(source: &str, cursor: usize) -> usize {
    let rope = Rope::from_str(source);
    let cursor = cursor.min(rope.len_chars());
    let line = rope.char_to_line(cursor);
    let line_start = rope.line_to_char(line);
    let mut start = cursor;
    while start > line_start {
        let character = rope.char(start - 1);
        if !(character == '_' || character.is_alphanumeric()) {
            break;
        }
        start -= 1;
    }
    start
}

fn lsp_position_offset(source: &str, position: &Value) -> Option<usize> {
    let line = position.get("line")?.as_u64()? as usize;
    let character = position.get("character")?.as_u64()? as usize;
    let rope = Rope::from_str(source);
    if line >= rope.len_lines() {
        return None;
    }
    let line_start = rope.line_to_char(line);
    let line_end = rope.line_to_char(line + 1).min(rope.len_chars());
    let mut utf16 = 0;
    for offset in line_start..line_end {
        if utf16 >= character {
            return Some(offset);
        }
        utf16 += rope.char(offset).len_utf16();
    }
    (utf16 >= character).then_some(line_end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_positions_convert_utf16_and_replace_prefix() {
        let source = "a😀b\nlet café = complete";
        assert_eq!(
            lsp_position_offset(source, &json!({"line": 0, "character": 3})),
            Some(2)
        );
        assert_eq!(completion_prefix_start(source, source.chars().count()), 15);
    }

    #[test]
    fn completion_items_use_server_text_edits() {
        let source = "fn main() { pri }";
        let response = json!({
            "result": {
                "items": [{
                    "label": "println!",
                    "textEdit": {
                        "newText": "println!(\"$0\")",
                        "range": {
                            "start": {"line": 0, "character": 12},
                            "end": {"line": 0, "character": 15}
                        }
                    }
                }]
            }
        });
        let items = parse_completions(&response, source, 15);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].insert_text, "println!(\"$0\")");
        assert_eq!((items[0].start, items[0].end), (12, 15));
    }
}
