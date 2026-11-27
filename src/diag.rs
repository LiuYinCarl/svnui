//! Optional diagnostic log (`--log-file PATH`).
//!
//! Deliberately minimal and dependency-free: svnui is a TUI, so bug reports
//! otherwise come with a screenshot and nothing else. The log records svn
//! subcommands, durations and failures — never commit messages, passwords
//! or full argument lists.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static LOG: OnceLock<Mutex<File>> = OnceLock::new();

/// Open (or create) the diagnostic log file.
pub fn init(path: &Path) -> Result<(), String> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("cannot open log file {}: {e}", path.display()))?;
    let _ = LOG.set(Mutex::new(file));
    log_line(&format!("svnui log started (pid {})", std::process::id()));
    Ok(())
}

/// Append one line; a no-op when `--log-file` was not given.
pub fn log_line(msg: &str) {
    let Some(log) = LOG.get() else {
        return;
    };
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut file) = log.lock() {
        let _ = writeln!(file, "[{secs}] {msg}");
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_writes_a_log_line() {
        let path = std::env::temp_dir().join(format!("svnui-diag-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        init(&path).expect("init");
        log_line("svn status ok 1ms");
        let text = std::fs::read_to_string(&path).expect("read log");
        assert!(text.contains("svnui log started"), "{text}");
        assert!(text.contains("svn status ok"), "{text}");
        let _ = std::fs::remove_file(&path);
    }
}
