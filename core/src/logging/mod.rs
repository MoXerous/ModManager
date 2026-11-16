//! Local logging: raw rolling activity log (F-149) and the shareable report
//! boundary (F-150).
//!
//! The activity log stays complete and unsanitized by design: it never leaves
//! the machine, and a local diagnosis needs every byte (F-149).
//!
//! `logging::report` is what "leaves the machine" means here: any file written
//! for someone else — a startup failure report today, a diagnostics export
//! tomorrow — goes through it, and every line is sanitized before the file is
//! created (F-150). A sanitisation failure aborts the report; the raw text is
//! never a fallback.

pub mod report;
pub mod sanitize;

pub use report::{write_report, write_report_with, ReportError};
pub use sanitize::{sanitize, sanitize_with, SanitizeError, UNAVAILABLE};

use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoggingError {
    #[error("io {op} on {}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Guard must be held for the whole process lifetime (drops flush the log).
pub struct LogGuard {
    _guard: tracing_appender::non_blocking::WorkerGuard,
}

/// Local activity log: raw rolling file, deliberately unsanitized (F-149).
pub fn init(dir: &Path) -> Result<LogGuard, LoggingError> {
    use tracing_subscriber::fmt::format::FmtSpan;
    std::fs::create_dir_all(dir).map_err(|e| LoggingError::Io {
        op: "create_dir",
        path: dir.to_path_buf(),
        source: e,
    })?;
    let appender = tracing_appender::rolling::daily(dir, "activity.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let subscriber = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_span_events(FmtSpan::NONE)
        .finish();
    tracing::subscriber::set_global_default(subscriber).map_err(|e| LoggingError::Io {
        op: "set_global_default",
        path: dir.to_path_buf(),
        source: std::io::Error::other(e.to_string()),
    })?;
    Ok(LogGuard { _guard: guard })
}

/// Records one line at error level, for a failure with no better channel.
///
/// The clients of this crate (the Tauri app, the CLI) do not depend on
/// `tracing` themselves: the log's shape stays defined in this one file, and
/// a message routed through here cannot drift from it. A no-op when [`init`]
/// never ran, which is exactly the case a caller reaching for it may be in.
pub fn error(message: impl std::fmt::Display) {
    tracing::error!("{message}");
}

#[cfg(test)]
mod tests {
    #[test]
    fn spec_f149_activity_log_keeps_raw_message() {
        let dir = crate::test_tempdir();
        let _guard = super::init(dir.path()).expect("init");
        tracing::info!("token=hunter2-local-value");
        drop(_guard);
        // Daily rotation names the file activity.log.<date>.
        let mut found = false;
        for e in std::fs::read_dir(dir.path()).expect("ls") {
            let p = e.expect("entry").path();
            if p.file_name()
                .map(|n| n.to_string_lossy().starts_with("activity.log"))
                .unwrap_or(false)
            {
                let log = std::fs::read_to_string(&p).expect("log file");
                assert!(
                    log.contains("hunter2-local-value"),
                    "raw log keeps all: {log}"
                );
                found = true;
            }
        }
        assert!(found, "a rolling log file must exist");
    }
}
