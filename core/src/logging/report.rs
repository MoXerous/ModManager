//! F-150: the fail-closed shareable report.
//!
//! Every line is sanitised before anything touches the disk, and a line that
//! cannot be sanitised aborts the whole report: no file, no raw fallback, no
//! partial file. The activity log stays raw and local (F-149); this is the
//! boundary a file crosses when it is meant to be sent to someone else.
//!
//! "No partial file" is enforced, not assumed: the report is published with a
//! rename, and a `.part` that cannot be removed after a failure is reported as
//! [`ReportError::Leftover`] instead of being dropped on the floor.

use std::path::{Path, PathBuf};

use thiserror::Error;

use super::sanitize::{sanitize, sanitize_with, SanitizeError};

/// F-150: why a report was not written.
///
/// Both variants mean the same thing to a caller: there is no report, and the
/// text that would have been in it was never written anywhere.
#[derive(Debug, Error)]
pub enum ReportError {
    /// One line is not safe to share, so none of them are written.
    #[error("line {line} of the report cannot be shared: {source}")]
    Sanitize {
        /// 1-based, so the number matches how a reader counts the lines.
        line: usize,
        #[source]
        source: SanitizeError,
    },
    /// The report could not be created or published.
    #[error("io {op} on {}: {source}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A `.part` file outlived a failed report, so "no partial file" is false.
    ///
    /// Removing the temp file can fail on its own (a scanner or indexer holding
    /// it open on Windows). Swallowing that left a half-written report beside
    /// the destination while this module claimed there was none; the leftover
    /// is now a typed failure that names the exact file, so the guarantee is
    /// either kept or honestly reported.
    #[error(
        "the partial report {} could not be removed after a failed {op}: {source}",
        path.display()
    )]
    Leftover {
        /// The operation that failed first (`write` or `rename`).
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// F-150: writes `lines` to `dir/name`, sanitised with the machine's policy.
///
/// This is the only sanctioned way to produce a file someone else may read:
/// the caller hands over the lines it wants to share and cannot bypass the
/// sanitiser. [`sanitize`] decides the policy (home paths and user names from
/// the environment).
pub fn write_report(dir: &Path, name: &str, lines: &[String]) -> Result<PathBuf, ReportError> {
    write_with(dir, name, lines, sanitize)
}

/// F-150: [`write_report`] with the policy injected (tests, and callers that
/// already resolved it).
pub fn write_report_with(
    dir: &Path,
    name: &str,
    lines: &[String],
    home_paths: &[String],
    usernames: &[String],
) -> Result<PathBuf, ReportError> {
    write_with(dir, name, lines, |line| {
        sanitize_with(line, home_paths, usernames)
    })
}

fn write_with(
    dir: &Path,
    name: &str,
    lines: &[String],
    sanitize_line: impl Fn(&str) -> Result<String, SanitizeError>,
) -> Result<PathBuf, ReportError> {
    // Every line is made shareable before anything is created: a failure here
    // leaves the destination exactly as it was.
    let mut body = String::new();
    for (index, line) in lines.iter().enumerate() {
        let safe = sanitize_line(line).map_err(|source| ReportError::Sanitize {
            line: index + 1,
            source,
        })?;
        body.push_str(&safe);
        body.push('\n');
    }

    std::fs::create_dir_all(dir).map_err(|source| io("create_dir", dir, source))?;
    let final_path = dir.join(name);
    // Same shape as the downloader: write beside the target, then publish with
    // a rename, so a reader never sees a half-written report.
    let part = dir.join(format!("{name}.part"));
    if let Err(source) = std::fs::write(&part, &body) {
        discard(&part, "write")?;
        return Err(io("write", &part, source));
    }
    if let Err(source) = std::fs::rename(&part, &final_path) {
        discard(&part, "rename")?;
        return Err(io("rename", &final_path, source));
    }
    Ok(final_path)
}

/// F-150: removes a `.part` that must not outlive a failed report.
///
/// A removal that fails is a failure of its own: the file is still there, and
/// "no partial file" would be a claim nobody checked.
fn discard(part: &Path, op: &'static str) -> Result<(), ReportError> {
    match std::fs::remove_file(part) {
        Ok(()) => Ok(()),
        // Nothing was created, so there is nothing to clean up.
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ReportError::Leftover {
            op,
            path: part.to_path_buf(),
            source,
        }),
    }
}

fn io(op: &'static str, path: &Path, source: std::io::Error) -> ReportError {
    ReportError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::sanitize::{REDACTED_CREDENTIAL, REDACTED_HOME, REDACTED_IP};

    fn policy() -> (Vec<String>, Vec<String>) {
        (
            vec!["C:\\Users\\Alice".to_string()],
            vec!["Alice".to_string()],
        )
    }

    /// F-150: a shareable report carries no secret and no identity.
    #[test]
    fn spec_f150_a_report_is_sanitised_before_it_is_written() {
        let dir = crate::test_tempdir();
        let (homes, users) = policy();
        let lines = vec![
            "app_version: 0.1.0".to_string(),
            "token: report-secret".to_string(),
            "home: C:\\Users\\Alice\\AppData".to_string(),
            "peer: 203.0.113.8:443".to_string(),
        ];

        let path = write_report_with(dir.path(), "support.txt", &lines, &homes, &users)
            .expect("the report sanitises");

        assert_eq!(path, dir.path().join("support.txt"));
        let written = std::fs::read_to_string(&path).expect("the report is on disk");
        assert!(
            written.contains(&format!("token: {REDACTED_CREDENTIAL}")),
            "{written}"
        );
        assert!(
            written.contains(&format!("home: {REDACTED_HOME}")),
            "{written}"
        );
        assert!(written.contains(REDACTED_IP), "{written}");
        assert!(written.contains("app_version: 0.1.0"), "{written}");
        for raw in ["report-secret", "Alice", "203.0.113.8"] {
            assert!(!written.contains(raw), "the report leaked {raw}: {written}");
        }
    }

    /// F-150: a report whose text cannot be sanitised leaves nothing behind.
    ///
    /// Not an empty file, not a raw one, not a `.part` of one: the failure has
    /// to be visible as the absence of the report.
    #[test]
    fn spec_f150_a_report_that_cannot_be_sanitised_leaves_no_file() {
        let dir = crate::test_tempdir();
        // The pass cannot reach a profile path with no trailing separator, so
        // this line is exactly the "cannot be redacted to a safe form" case.
        let (homes, users) = (Vec::new(), vec!["Alice".to_string()]);
        let lines = vec![
            "app_version: 0.1.0".to_string(),
            "home is C:\\Users\\Alice".to_string(),
        ];

        let error = write_report_with(dir.path(), "blocked.txt", &lines, &homes, &users)
            .expect_err("the line cannot be sanitised");

        assert!(
            matches!(error, ReportError::Sanitize { line: 2, .. }),
            "the failing line has to be named: {error}"
        );
        assert!(
            !dir.path().join("blocked.txt").exists(),
            "no report may be written"
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("ls")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert!(
            leftovers.is_empty(),
            "nothing raw may be left: {leftovers:?}"
        );
    }

    /// F-150: a short account name does not stop the report from being written.
    ///
    /// A two-letter `USERNAME` used to be rejected as an invalid policy, so the
    /// file was never created and nothing said why. Per-value policy: the name
    /// is simply not applied when it does not appear.
    #[test]
    fn spec_f150_a_short_user_name_does_not_block_the_report() {
        let dir = crate::test_tempdir();
        let lines = vec![
            "app_version: 0.1.0".to_string(),
            "os: windows x86_64".to_string(),
        ];

        let path = write_report_with(dir.path(), "short.txt", &lines, &[], &["jo".to_string()])
            .expect("a short user name is not a policy failure");

        assert_eq!(path, dir.path().join("short.txt"));
        assert!(path.exists(), "the report has to exist");
    }

    /// F-150: a failed rename leaves no `.part` behind — or says that it did.
    ///
    /// The report is published by renaming a `.part` file, so a failure there
    /// must not leave the half-written file where a user could pick it up.
    #[test]
    fn spec_f150_a_failed_rename_leaves_no_part_file() {
        let dir = crate::test_tempdir();
        // A directory where the report wants to be: the rename fails, the
        // write before it succeeded.
        std::fs::create_dir(dir.path().join("blocked.txt")).expect("a blocked target");

        let error = write_report_with(
            dir.path(),
            "blocked.txt",
            &["app_version: 0.1.0".to_string()],
            &[],
            &[],
        )
        .expect_err("the target is a directory");

        assert!(
            matches!(error, ReportError::Io { op: "rename", .. }),
            "the failed operation has to be named: {error}"
        );
        assert!(
            !dir.path().join("blocked.txt.part").exists(),
            "a .part file outlived the failed report"
        );
    }

    /// F-150: a `.part` that cannot be removed is reported, not swallowed.
    ///
    /// `let _ = remove_file(..)` is how a locked file (an antivirus scanner on
    /// Windows) leaves a partial report behind while the module claims there is
    /// none. The leftover is a typed failure with the path on it.
    #[test]
    fn spec_f150_a_part_file_that_cannot_be_removed_is_reported() {
        let dir = crate::test_tempdir();
        // A directory cannot be removed with `remove_file` on any platform,
        // which is the shape of a removal that fails.
        let part = dir.path().join("leftover.txt.part");
        std::fs::create_dir(&part).expect("a directory at the .part path");

        let error = discard(&part, "rename").expect_err("the leftover cannot be removed");

        assert!(
            matches!(error, ReportError::Leftover { op: "rename", .. }),
            "the leftover has to be a typed failure: {error}"
        );
        assert!(
            error.to_string().contains("leftover.txt.part"),
            "the path has to be in the message: {error}"
        );
    }
}
