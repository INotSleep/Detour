//! The log: errors of the core, the window and the command line, and what crashed, so a
//! problem can be traced afterwards. One file in the data folder (`logs/detour.log`); when it
//! grows past a limit it becomes `detour.1.log` and a new one starts.
//!
//! Only errors and the core's start go in: never what is typed into terminals, never a
//! secret (calls are logged by their method and error, not their arguments).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};
use std::time::{SystemTime, UNIX_EPOCH};

/// How big the file grows before it is set aside.
const MAX: u64 = 2 * 1024 * 1024;

struct Log {
    path: PathBuf,
    /// Which program writes: core, app, cli.
    who: &'static str,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);
static PANICS: Once = Once::new();

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

pub fn path(data_dir: &Path) -> PathBuf {
    dir(data_dir).join("detour.log")
}

/// Logs this process's errors as `who`, and what makes it crash (with a backtrace).
pub fn init(data_dir: &Path, who: &'static str) {
    *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(Log { path: path(data_dir), who });
    PANICS.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            write("panic", &format!("{info}\n{}", std::backtrace::Backtrace::force_capture()));
            before(info);
        }));
    });
}

/// Adds a line (more lines are indented under it). Never fails: a log that can't be written is skipped.
pub fn write(level: &str, message: &str) {
    let guard = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let Some(log) = guard.as_ref() else { return };
    let _ = append(log, level, message);
}

pub fn error(message: &str) {
    write("error", message);
}

pub fn info(message: &str) {
    write("info", message);
}

fn append(log: &Log, level: &str, message: &str) -> std::io::Result<()> {
    if let Some(dir) = log.path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(&log.path).is_ok_and(|m| m.len() > MAX) {
        let _ = std::fs::rename(&log.path, log.path.with_file_name("detour.1.log"));
    }
    let body = message.trim_end().replace('\n', "\n    ");
    let line = format!("{} {level:<5} {}[{}] {body}\n", timestamp(), log.who, std::process::id());
    std::fs::OpenOptions::new().create(true).append(true).open(&log.path)?.write_all(line.as_bytes())
}

/// The end of the log (up to `max` bytes, from a line's start), for the window to show or copy.
pub fn tail(data_dir: &Path, max: usize) -> String {
    let text = std::fs::read(path(data_dir)).unwrap_or_default();
    let from = text.len().saturating_sub(max);
    let text = String::from_utf8_lossy(&text[from..]).to_string();
    match (from, text.find('\n')) {
        (0, _) | (_, None) => text,
        (_, Some(i)) => text[i + 1..].to_string(),
    }
}

/// UTC, as `2026-11-14 12:03:04Z`.
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_go_in_and_the_end_comes_back() {
        let dir = std::env::temp_dir().join(format!("detour-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        init(&dir, "test");
        error("it broke\nat line 2");
        info("started");
        let text = tail(&dir, 1 << 20);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(lines[0].contains(" error test[") && lines[0].ends_with("it broke"), "{text}");
        assert_eq!(lines[1], "    at line 2");
        assert!(lines[2].ends_with("] started"));
        assert_eq!(&lines[0][4..5], "-");
        // From the middle of a line, the tail starts at the next one.
        assert!(tail(&dir, text.len() - 5).starts_with("    at line 2\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn timestamps_are_utc_dates() {
        let t = timestamp();
        assert_eq!((t.len(), &t[10..11], &t[19..]), (20, " ", "Z"), "{t}");
        assert!(t.starts_with("20"));
    }
}
