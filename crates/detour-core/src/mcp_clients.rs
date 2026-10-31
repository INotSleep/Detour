//! Detour's MCP server (`detour mcp`) in the configs of AI clients: which are on this
//! computer, which already have it, and adding or removing it there.
//!
//! Each client keeps its servers in a file of its own. JSON files are read and written
//! whole (a file with comments is left alone: it can't be edited without losing them);
//! Codex's TOML is edited as text, so the rest of it stays as it was. Claude Code is told
//! through its own `claude mcp` when it is installed, as it rewrites its file while it runs.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::{Error, Result};

/// The name the server has in every client.
const NAME: &str = "detour";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// `{ "mcpServers": { name: { command, args } } }`
    McpServers,
    /// The same, with `"type": "stdio"` (Claude Code's own file).
    McpServersTyped,
    /// VS Code: `{ "servers": { name: { type, command, args } } }`
    Servers,
    /// Codex: `[mcp_servers.name]` in TOML.
    CodexToml,
}

struct Spec {
    id: &'static str,
    name: &'static str,
    format: Format,
    /// The config file, from the home and config directories.
    file: fn(&Dirs) -> PathBuf,
    /// Something that is there when the client is installed.
    marker: fn(&Dirs) -> PathBuf,
}

/// Where a user's files are; tests give their own.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub home: PathBuf,
    /// `%APPDATA%`, `~/Library/Application Support`, `~/.config`.
    pub config: PathBuf,
}

impl Dirs {
    pub fn of_user() -> Option<Self> {
        let d = directories::BaseDirs::new()?;
        Some(Self { home: d.home_dir().to_path_buf(), config: d.config_dir().to_path_buf() })
    }
}

const CLIENTS: &[Spec] = &[
    Spec { id: "claude-code", name: "Claude Code", format: Format::McpServersTyped, file: |d| d.home.join(".claude.json"), marker: |d| d.home.join(".claude") },
    Spec { id: "claude-desktop", name: "Claude Desktop", format: Format::McpServers, file: |d| d.config.join("Claude").join("claude_desktop_config.json"), marker: |d| d.config.join("Claude") },
    Spec { id: "codex", name: "Codex", format: Format::CodexToml, file: |d| d.home.join(".codex").join("config.toml"), marker: |d| d.home.join(".codex") },
    Spec { id: "cursor", name: "Cursor", format: Format::McpServers, file: |d| d.home.join(".cursor").join("mcp.json"), marker: |d| d.home.join(".cursor") },
    Spec { id: "vscode", name: "VS Code", format: Format::Servers, file: |d| d.config.join("Code").join("User").join("mcp.json"), marker: |d| d.config.join("Code").join("User") },
    Spec { id: "windsurf", name: "Windsurf", format: Format::McpServers, file: |d| d.home.join(".codeium").join("windsurf").join("mcp_config.json"), marker: |d| d.home.join(".codeium").join("windsurf") },
    Spec { id: "gemini", name: "Gemini CLI", format: Format::McpServers, file: |d| d.home.join(".gemini").join("settings.json"), marker: |d| d.home.join(".gemini") },
];

/// A client as the app and `detour mcp clients` show it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientState {
    pub id: &'static str,
    pub name: &'static str,
    pub installed: bool,
    /// It has a `detour` server.
    pub added: bool,
    /// ...that runs this program with these arguments (otherwise it is an older one, worth replacing).
    pub current: bool,
    pub config: String,
}

/// The server as clients start it: this `detour` and its arguments.
#[derive(Debug, Clone)]
pub struct Server {
    pub command: PathBuf,
    pub args: Vec<String>,
}

impl Server {
    /// `detour mcp`, for a vault folder other than the default one when given.
    pub fn new(command: PathBuf, data_dir: Option<&Path>) -> Self {
        let mut args = Vec::new();
        if let Some(d) = data_dir {
            args.extend(["--data-dir".to_string(), d.display().to_string()]);
        }
        args.push("mcp".into());
        Self { command, args }
    }
}

fn io(path: &Path, e: impl std::fmt::Display) -> Error {
    Error::Io { message: format!("{}: {e}", path.display()) }
}

fn spec(id: &str) -> Result<&'static Spec> {
    CLIENTS.iter().find(|s| s.id == id).ok_or_else(|| Error::NotFound { what: format!("MCP client {id}") })
}

/* ---------- JSON files ---------- */

fn read_json(path: &Path) -> Result<Value> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({})),
        Err(e) => return Err(io(path, e)),
    };
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    // A file with comments or trailing commas isn't JSON: better left for the person to edit.
    serde_json::from_str(&text).map_err(|e| io(path, e))
}

fn write_json(path: &Path, v: &Value) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    }
    let text = serde_json::to_string_pretty(v)? + "\n";
    std::fs::write(path, text).map_err(|e| io(path, e))
}

fn servers_key(f: Format) -> &'static str {
    if f == Format::Servers { "servers" } else { "mcpServers" }
}

fn json_entry(f: Format, s: &Server) -> Value {
    let base = json!({ "command": s.command.display().to_string(), "args": s.args });
    match f {
        Format::McpServers => base,
        _ => {
            let mut e = json!({ "type": "stdio" });
            e.as_object_mut().unwrap().extend(base.as_object().unwrap().clone());
            e
        }
    }
}

fn json_current(entry: &Value, s: &Server) -> bool {
    let command = entry["command"].as_str().map(PathBuf::from);
    let args: Vec<String> = entry["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_string)).collect();
    command.as_deref() == Some(s.command.as_path()) && args == s.args
}

/* ---------- Codex's TOML ---------- */

/// A TOML string: a literal one when it can be (Windows paths keep their backslashes).
fn toml_str(s: &str) -> String {
    if s.contains('\'') || s.contains('\n') { serde_json::to_string(s).unwrap_or_default() } else { format!("'{s}'") }
}

fn is_header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

/// Is this header the server's table, or one of its sub-tables (`.env`)?
fn ours(line: &str) -> bool {
    let h = line.trim();
    h == "[mcp_servers.detour]" || h.starts_with("[mcp_servers.detour.")
}

/// The text without the server's tables; `None` when it had none.
fn toml_without(text: &str) -> Option<String> {
    let mut out = Vec::new();
    let (mut skipping, mut found) = (false, false);
    for line in text.lines() {
        if is_header(line) {
            skipping = ours(line);
            found |= skipping;
        }
        if !skipping {
            out.push(line);
        }
    }
    found.then(|| {
        let mut s = out.join("\n").trim_end().to_string();
        if !s.is_empty() {
            s.push('\n');
        }
        s
    })
}

fn toml_block(s: &Server) -> String {
    let args: Vec<String> = s.args.iter().map(|a| toml_str(a)).collect();
    format!("[mcp_servers.{NAME}]\ncommand = {}\nargs = [{}]\n", toml_str(&s.command.display().to_string()), args.join(", "))
}

fn toml_current(text: &str, s: &Server) -> bool {
    text.contains(&toml_block(s))
}

fn read_text(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(io(path, e)),
    }
}

/* ---------- Claude Code's own command ---------- */

/// `claude` on the PATH, if Claude Code's command line is installed.
fn claude_cli() -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd"] } else { &[""] };
    std::env::split_paths(&std::env::var_os("PATH")?).flat_map(|dir| exts.iter().map(move |e| dir.join(format!("claude{e}")))).find(|p| p.is_file())
}

fn run_claude(cli: &Path, args: &[String]) -> Result<std::process::Output> {
    let mut cmd = if cli.extension().is_some_and(|e| e == "cmd") {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(cli);
        c
    } else {
        Command::new(cli)
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up from the app.
        cmd.creation_flags(0x0800_0000);
    }
    cmd.args(args).output().map_err(|e| io(cli, e))
}

/* ---------- what the app and the command line call ---------- */

/// Every client Detour knows, installed or not, and whether it has the server.
pub fn clients(dirs: &Dirs, server: &Server) -> Vec<ClientState> {
    CLIENTS
        .iter()
        .map(|c| {
            let file = (c.file)(dirs);
            let (added, current) = match c.format {
                Format::CodexToml => {
                    let text = read_text(&file).unwrap_or_default();
                    (text.lines().any(|l| l.trim() == format!("[mcp_servers.{NAME}]")), toml_current(&text, server))
                }
                f => match read_json(&file).ok().map(|v| v[servers_key(f)][NAME].clone()).filter(|e| !e.is_null()) {
                    Some(e) => (true, json_current(&e, server)),
                    None => (false, false),
                },
            };
            ClientState { id: c.id, name: c.name, installed: (c.marker)(dirs).exists() || file.exists(), added, current, config: file.display().to_string() }
        })
        .collect()
}

/// Adds the server to a client (replacing an older one); `use_cli` lets Claude Code's own command do it.
pub fn add(dirs: &Dirs, id: &str, server: &Server, use_cli: bool) -> Result<()> {
    let c = spec(id)?;
    let file = (c.file)(dirs);
    if c.id == "claude-code"
        && use_cli
        && let Some(cli) = claude_cli()
    {
        let _ = run_claude(&cli, &["mcp".into(), "remove".into(), "--scope".into(), "user".into(), NAME.into()]);
        let mut args = vec!["mcp".into(), "add".into(), "--scope".into(), "user".into(), NAME.into(), "--".into(), server.command.display().to_string()];
        args.extend(server.args.iter().cloned());
        let out = run_claude(&cli, &args)?;
        if !out.status.success() {
            return Err(Error::Io { message: format!("claude mcp add: {}", String::from_utf8_lossy(&out.stderr).trim()) });
        }
        return Ok(());
    }
    match c.format {
        Format::CodexToml => {
            let text = read_text(&file)?;
            let mut text = toml_without(&text).unwrap_or(text);
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&toml_block(server));
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
            }
            std::fs::write(&file, text).map_err(|e| io(&file, e))
        }
        f => {
            let mut v = read_json(&file)?;
            let root = v.as_object_mut().ok_or_else(|| io(&file, "not a JSON object"))?;
            let servers = root.entry(servers_key(f)).or_insert_with(|| Value::Object(Map::new()));
            servers.as_object_mut().ok_or_else(|| io(&file, format!("{} is not an object", servers_key(f))))?.insert(NAME.into(), json_entry(f, server));
            write_json(&file, &v)
        }
    }
}

/// Removes the server from a client; `false` when it had none.
pub fn remove(dirs: &Dirs, id: &str, use_cli: bool) -> Result<bool> {
    let c = spec(id)?;
    let file = (c.file)(dirs);
    if c.id == "claude-code"
        && use_cli
        && let Some(cli) = claude_cli()
    {
        let out = run_claude(&cli, &["mcp".into(), "remove".into(), "--scope".into(), "user".into(), NAME.into()])?;
        return Ok(out.status.success());
    }
    match c.format {
        Format::CodexToml => match toml_without(&read_text(&file)?) {
            Some(text) => std::fs::write(&file, text).map(|_| true).map_err(|e| io(&file, e)),
            None => Ok(false),
        },
        f => {
            let mut v = read_json(&file)?;
            let removed = v.get_mut(servers_key(f)).and_then(Value::as_object_mut).is_some_and(|m| m.remove(NAME).is_some());
            if removed {
                write_json(&file, &v)?;
            }
            Ok(removed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> Dirs {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("detour-mcp-clients-{}-{nanos}", std::process::id()));
        Dirs { home: root.join("home"), config: root.join("config") }
    }

    fn server() -> Server {
        Server::new(PathBuf::from(r"C:\Program Files\Detour\detour.exe"), Some(Path::new(r"C:\Users\me\vault")))
    }

    #[test]
    fn json_clients_keep_everything_else() {
        let d = temp();
        let file = d.config.join("Code").join("User").join("mcp.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{ "servers": { "other": { "command": "x" } }, "inputs": [1] }"#).unwrap();

        let state = |d: &Dirs| clients(d, &server()).into_iter().find(|c| c.id == "vscode").unwrap();
        assert!(state(&d).installed && !state(&d).added);
        add(&d, "vscode", &server(), false).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["servers"]["detour"], json!({ "type": "stdio", "command": r"C:\Program Files\Detour\detour.exe", "args": ["--data-dir", r"C:\Users\me\vault", "mcp"] }));
        assert_eq!((v["servers"]["other"]["command"].clone(), v["inputs"].clone()), (json!("x"), json!([1])));
        assert!(state(&d).added && state(&d).current);

        // Moved elsewhere: the old entry is not current, adding again replaces it.
        let moved = Server::new(PathBuf::from("/usr/bin/detour"), None);
        assert!(!clients(&d, &moved).into_iter().find(|c| c.id == "vscode").unwrap().current);

        assert!(remove(&d, "vscode", false).unwrap());
        assert!(!remove(&d, "vscode", false).unwrap());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["servers"], json!({ "other": { "command": "x" } }));
    }

    #[test]
    fn a_file_with_comments_is_left_alone() {
        let d = temp();
        let file = d.home.join(".cursor").join("mcp.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let text = "{\n  // mine\n  \"mcpServers\": {}\n}\n";
        std::fs::write(&file, text).unwrap();
        assert!(add(&d, "cursor", &server(), false).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    }

    #[test]
    fn codex_toml_is_edited_as_text() {
        let d = temp();
        let file = d.home.join(".codex").join("config.toml");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let before = "model = \"x\" # mine\n\n[mcp_servers.node]\ncommand = \"node\"\n\n[projects.'c:\\a']\ntrust_level = \"trusted\"\n";
        std::fs::write(&file, before).unwrap();

        add(&d, "codex", &server(), false).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with(before), "{text}");
        assert!(text.ends_with("[mcp_servers.detour]\ncommand = 'C:\\Program Files\\Detour\\detour.exe'\nargs = ['--data-dir', 'C:\\Users\\me\\vault', 'mcp']\n"), "{text}");
        let state = clients(&d, &server()).into_iter().find(|c| c.id == "codex").unwrap();
        assert!(state.added && state.current);

        // Again (an update) keeps one table; with an env table of its own, removing takes both.
        add(&d, "codex", &server(), false).unwrap();
        let mut text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.matches("[mcp_servers.detour]").count(), 1);
        text.push_str("[mcp_servers.detour.env]\nX = \"1\"\n");
        std::fs::write(&file, text).unwrap();
        assert!(remove(&d, "codex", false).unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    }

    #[test]
    fn clients_not_installed_say_so() {
        let d = temp();
        assert!(clients(&d, &server()).iter().all(|c| !c.installed && !c.added));
        assert!(add(&d, "nope", &server(), false).is_err());
    }
}
