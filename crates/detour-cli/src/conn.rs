//! Reaching the core from a terminal: start it if needed, make this terminal
//! trusted (the window's "Allow", or the master password typed here) and unlocked.

use std::io::{IsTerminal, Write};
use std::path::Path;

use detour_ipc::Client;
use serde_json::{Value, json};

use crate::i18n::{error_text, t};

/// What a command ends with when it fails: a message for stderr and an exit code.
pub struct Fail(pub String);

pub type Res<T> = Result<T, Fail>;

impl From<Value> for Fail {
    fn from(e: Value) -> Self {
        Fail(error_text(&e))
    }
}

pub fn fail<T>(msg: impl Into<String>) -> Res<T> {
    Err(Fail(msg.into()))
}

/// Asks for a secret on the terminal itself, never on a pipe: an agent can't answer.
pub fn secret(prompt: &str) -> Res<String> {
    if !std::io::stdin().is_terminal() {
        return fail(t().needs_terminal);
    }
    // The prompt goes through Rust's own console output: rpassword writes it as raw bytes,
    // which a Windows console in its OEM code page shows as mojibake.
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    rpassword::read_password().map_err(|e| Fail(e.to_string()))
}

/// A yes/no question on the terminal; anything but "y" is no.
pub fn confirm(prompt: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).is_ok() && matches!(line.trim(), "y" | "Y" | "yes" | "д" | "Д" | "да")
}

/// A connection to the core, started when none runs.
pub async fn connect(data_dir: &Path) -> Res<Client> {
    let me = std::env::current_exe().map_err(|e| Fail(e.to_string()))?;
    let c = detour_ipc::connect_or_start(&me, data_dir).await.map_err(|e| Fail(format!("{}: {e}", t().core_unreachable)))?;
    c.call("hello", json!({ "client": "cli" })).await?;
    Ok(c)
}

/// A connection that may do what the person can: trusted and unlocked.
pub async fn ready(data_dir: &Path) -> Res<Client> {
    let c = connect(data_dir).await?;
    let status = c.call("vault_status", json!({})).await?;
    if status["initialized"] == false {
        if !confirm(t().no_vault) {
            return fail(t().cancelled);
        }
        let pw = secret(t().new_master_password)?;
        if secret(t().repeat_password)? != pw {
            return fail(t().passwords_differ);
        }
        c.call("vault_create", json!({ "masterPassword": pw })).await?;
        return Ok(c);
    }
    match c.call("vault_get", json!({})).await {
        Ok(_) => return Ok(c),
        // The window is open: it is asked; this terminal waits for the answer.
        Err(e) if e["kind"] == "notTrusted" && e["window"] == true => {
            eprintln!("{}", t().allow_in_window);
            match c.call("trust_request", json!({})).await {
                Ok(_) => {}
                Err(e) if e["kind"] == "needMasterPassword" => unlock_with_password(&c).await?,
                Err(e) => return Err(e.into()),
            }
        }
        Err(e) if e["kind"] == "notTrusted" || e["kind"] == "locked" => unlock_with_password(&c).await?,
        Err(e) => return Err(e.into()),
    }
    // Allowed in the window while the vault is locked: it still needs the password.
    if c.call("vault_status", json!({})).await?["unlocked"] == false {
        unlock_with_password(&c).await?;
    }
    Ok(c)
}

async fn unlock_with_password(c: &Client) -> Res<()> {
    for attempt in 0..3 {
        let pw = secret(t().master_password)?;
        match c.call("trust_with_password", json!({ "masterPassword": pw })).await {
            Ok(_) => return Ok(()),
            Err(e) if e["kind"] == "wrongMasterPassword" && attempt < 2 => eprintln!("{}", error_text(&e)),
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}
