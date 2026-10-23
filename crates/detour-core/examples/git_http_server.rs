//! Local Git host over HTTP for trying team vaults by hand.
//!
//! cargo run -p detour-core --example git_http_server -- <repos folder> [port] [token]
//!
//! Serves the bare repositories in a folder through the installed `git
//! http-backend`, like a Git host, at `http://127.0.0.1:<port>/<name>.git`.
//! Every request must carry the token (default `test-token`) as a password.

use std::path::{Path, PathBuf};

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let root: PathBuf = args.next().expect("folder with bare repositories").into();
    let port: u16 = args.next().map(|p| p.parse().expect("port")).unwrap_or(8080);
    let token = args.next().unwrap_or_else(|| "test-token".into());
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    println!("serving {} on http://127.0.0.1:{port}/, token {token}", root.display());
    while let Ok((sock, _)) = listener.accept().await {
        let (root, token) = (root.clone(), token.clone());
        tokio::spawn(async move {
            if let Err(e) = serve(sock, &root, &token).await {
                eprintln!("{e}");
            }
        });
    }
}

async fn serve(mut sock: TcpStream, root: &Path, token: &str) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut first = head.lines().next().unwrap_or_default().split(' ');
    let (method, target) = (first.next().unwrap_or_default().to_string(), first.next().unwrap_or_default().to_string());
    let header = |name: &str| head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()));
    let length: usize = header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < length {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }

    // Any login name, the right token.
    let password = header("authorization")
        .and_then(|h| h.strip_prefix("Basic ").map(str::to_string))
        .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
        .and_then(|d| String::from_utf8(d).ok())
        .and_then(|d| d.split_once(':').map(|(_, p)| p.to_string()));
    println!("{method} {target} {}", if password.as_deref() == Some(token) { "" } else { "(refused)" });
    if password.as_deref() != Some(token) {
        sock.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"git\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
        return Ok(());
    }

    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let mut child = tokio::process::Command::new("git")
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REQUEST_METHOD", &method)
        .env("PATH_INFO", path)
        .env("QUERY_STRING", query)
        .env("CONTENT_TYPE", header("content-type").unwrap_or_default())
        .env("CONTENT_LENGTH", body.len().to_string())
        .env("REMOTE_USER", "detour")
        .env("REMOTE_ADDR", "127.0.0.1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&body).await?;
    drop(stdin);
    let out = child.wait_with_output().await?.stdout;

    // CGI output: headers, a blank line, the body.
    let (end, skip) = out.windows(4).position(|w| w == b"\r\n\r\n").map(|i| (i, 4)).or_else(|| out.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2))).unwrap_or((out.len(), 0));
    let cgi_head = String::from_utf8_lossy(&out[..end]).to_string();
    let cgi_body = &out[(end + skip).min(out.len())..];
    let mut status = "200 OK".to_string();
    let mut headers = String::new();
    for l in cgi_head.lines() {
        match l.split_once(':') {
            Some((k, v)) if k.eq_ignore_ascii_case("status") => status = v.trim().to_string(),
            Some((k, v)) => headers.push_str(&format!("{k}: {}\r\n", v.trim())),
            None => {}
        }
    }
    sock.write_all(format!("HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n", cgi_body.len()).as_bytes()).await?;
    sock.write_all(cgi_body).await
}
