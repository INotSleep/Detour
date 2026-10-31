//! `detour ssh`: a terminal session in this terminal, like `ssh`. Keys go to the
//! server as they are typed, the window size follows, the exit status is the server's.

use std::io::{Read, Write};
use std::time::Duration;

use detour_ipc::StreamItem;
use serde_json::{Value, json};

use crate::cmds::{Ctx, host, with_login};
use crate::conn::{Fail, Res};

/// Puts the terminal back as it was, however the session ends.
struct Raw {
    #[cfg(windows)]
    modes: (u32, u32),
}

impl Raw {
    fn on() -> Res<Self> {
        crossterm::terminal::enable_raw_mode().map_err(|e| Fail(e.to_string()))?;
        // Keys arrive as the escape sequences a server expects, and its output is drawn the same way.
        #[cfg(windows)]
        // SAFETY: the standard handles of this process, and modes read from them first.
        let modes = unsafe {
            use windows_sys::Win32::System::Console::*;
            let (inp, out) = (GetStdHandle(STD_INPUT_HANDLE), GetStdHandle(STD_OUTPUT_HANDLE));
            let (mut mi, mut mo) = (0u32, 0u32);
            GetConsoleMode(inp, &mut mi);
            GetConsoleMode(out, &mut mo);
            SetConsoleMode(inp, mi | ENABLE_VIRTUAL_TERMINAL_INPUT);
            SetConsoleMode(out, mo | ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN);
            (mi, mo)
        };
        Ok(Self {
            #[cfg(windows)]
            modes,
        })
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Console::*;
            // SAFETY: the modes these handles had before.
            unsafe {
                SetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), self.modes.0);
                SetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), self.modes.1);
            }
        }
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn size() -> (u16, u16) {
    crossterm::terminal::size().unwrap_or((80, 24))
}

pub async fn ssh(ctx: &Ctx, name: &str) -> Res<i32> {
    let h = host(ctx, name).await?;
    let (stream, mut rx) = ctx.c.stream();
    let (c, vault, host_id) = (ctx.c.clone(), ctx.vault.clone(), h["id"].clone());
    let opened = with_login(ctx, &h, |secret, trust| {
        let (c, vault, host_id) = (c.clone(), vault.clone(), host_id.clone());
        let (cols, rows) = size();
        // The session ends with this command: it is not one the window keeps.
        async move {
            let args = json!({ "vaultId": vault, "hostId": host_id, "secret": secret, "trustFingerprint": trust, "cols": cols, "rows": rows });
            c.call("session_open", json!({ "args": args, "stream": stream, "closeWithClient": true })).await
        }
    })
    .await?;
    let session = opened["sessionId"].clone();
    let raw = Raw::on()?;

    // Keys: read on a thread of their own, as reading blocks.
    let (keys_tx, mut keys) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 || keys_tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let mut out = std::io::stdout();
    let mut last = size();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let status = loop {
        tokio::select! {
            item = rx.recv() => match item {
                Some(StreamItem::Data(b)) => { let _ = out.write_all(&b); let _ = out.flush(); }
                Some(StreamItem::Json(v)) if v.get("closed").is_some() => break v["exitStatus"].clone(),
                Some(StreamItem::Json(_)) => {}
                None => break Value::Null,
            },
            Some(k) = keys.recv() => {
                let _ = ctx.c.call("session_input", json!({ "sessionId": session, "data": String::from_utf8_lossy(&k) })).await;
            }
            _ = tick.tick() => {
                let now = size();
                if now != last {
                    last = now;
                    let _ = ctx.c.call("session_resize", json!({ "sessionId": session, "cols": now.0, "rows": now.1 })).await;
                }
            }
        }
    };
    drop(raw);
    Ok(status.as_i64().map(|s| s as i32).unwrap_or(255))
}
