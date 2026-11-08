//! CurseForge API key file (ADR 0004 §2-1).
//!
//! The key lives in its own JSON file under `config_dir`, never in
//! `settings.json`: file first, then `CURSEFORGE_API_KEY`, then none. A file
//! that exists but is corrupt is a typed error — silently falling back to the
//! environment would show `file` in Settings while using another key.
//!
//! Writes copy the journal's protocol (ADR §2-1-3): a unique
//! `.<name>.<uuid>.tmp` with `create_new(true)`, `sync_all`, then
//! `safety::atomic::commit_part`. Deliberately **not** `write_atomic`: its
//! fixed `.part` is never deleted after a crash, which would make every later
//! save impossible. The temporary is removed on any failure here.

use crate::paths::Paths;
use crate::safety::atomic;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// `config_dir/curseforge.secret.json` (ADR §2-1-1).
pub const SECRET_FILE_NAME: &str = "curseforge.secret.json";
/// The key is tens of bytes; 4 KiB refuses an accidental huge file
/// *before* reading it (ADR §2-1-1).
pub const SECRET_MAX_BYTES: u64 = 4 * 1024;
pub const SECRET_SCHEMA_VERSION: u32 = 1;

/// Where the live key came from. Never carries the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    File,
    Environment,
    None,
}

/// What Settings is allowed to show (ADR §2-1-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyStatus {
    pub source: KeySource,
    pub has_key: bool,
}

#[derive(Debug, Error)]
pub enum SecretError {
    /// The write protocol refuses a symlinked target, like the journal's
    /// record writer (`journal/store/mod.rs:159-163`).
    #[error("curseforge secret path is a symlink")]
    Symlink,
    #[error("curseforge secret file is larger than {max} bytes")]
    TooLarge { max: u64 },
    #[error("curseforge secret file is not valid JSON")]
    InvalidJson,
    #[error("curseforge secret file has unsupported schema version {found}")]
    UnsupportedSchema { found: u32 },
    #[error("curseforge secret key is empty")]
    EmptyKey,
    #[error("curseforge secret path has no parent directory")]
    NoParent,
    #[error("io {op} on {}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not commit the curseforge secret file")]
    Commit(#[from] atomic::AtomicError),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretFile {
    schema_version: u32,
    api_key: String,
}

pub fn secret_path(paths: &Paths) -> PathBuf {
    paths.config_dir().join(SECRET_FILE_NAME)
}

/// Read the key from `path`, falling back to `env` only when the file is
/// absent. `env` is passed explicitly so tests never mutate the process
/// environment. Callers supply `std::env::var("CURSEFORGE_API_KEY").ok()`.
pub fn load_key(
    path: &Path,
    env: Option<&str>,
) -> Result<(Option<String>, KeyStatus), SecretError> {
    match read_file(path)? {
        Some(key) => Ok((
            Some(key),
            KeyStatus {
                source: KeySource::File,
                has_key: true,
            },
        )),
        None => match env {
            Some(value) if !value.trim().is_empty() => Ok((
                Some(value.to_string()),
                KeyStatus {
                    source: KeySource::Environment,
                    has_key: true,
                },
            )),
            _ => Ok((
                None,
                KeyStatus {
                    source: KeySource::None,
                    has_key: false,
                },
            )),
        },
    }
}

/// Status only: the caller (IPC/Settings) never receives the value.
/// A corrupt file still fails here; it is not reported as `none`.
pub fn key_status(path: &Path, env: Option<&str>) -> Result<KeyStatus, SecretError> {
    load_key(path, env).map(|(_, status)| status)
}

/// Atomically write `key` to `path`.
///
/// A stale temp from a crashed attempt has a different UUID, so it can never
/// block this write; only the target is replaced.
pub fn save_key(path: &Path, key: &str) -> Result<(), SecretError> {
    let key = trim_one_newline(key);
    if key.trim().is_empty() {
        return Err(SecretError::EmptyKey);
    }
    let bytes = serde_json::to_vec(&SecretFile {
        schema_version: SECRET_SCHEMA_VERSION,
        api_key: key.to_string(),
    })
    .map_err(|_| SecretError::InvalidJson)?;
    if bytes.len() as u64 > SECRET_MAX_BYTES {
        return Err(SecretError::TooLarge {
            max: SECRET_MAX_BYTES,
        });
    }

    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(SecretError::Symlink);
        }
    }

    let dir = path.parent().ok_or(SecretError::NoParent)?;
    let name = path
        .file_name()
        .ok_or(SecretError::NoParent)?
        .to_string_lossy();
    let temp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));

    let result = write_temp(&temp, &bytes)
        .and_then(|()| atomic::commit_part(&temp, path).map_err(SecretError::from));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result?;
    sync_dir_best_effort(dir);
    Ok(())
}

fn read_file(path: &Path) -> Result<Option<String>, SecretError> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err("stat", path, e)),
    };
    if meta.file_type().is_symlink() {
        return Err(SecretError::Symlink);
    }
    if meta.len() > SECRET_MAX_BYTES {
        return Err(SecretError::TooLarge {
            max: SECRET_MAX_BYTES,
        });
    }
    let bytes = std::fs::read(path).map_err(|e| io_err("read", path, e))?;
    // The file may have grown between stat and read; keep the same ceiling.
    if bytes.len() as u64 > SECRET_MAX_BYTES {
        return Err(SecretError::TooLarge {
            max: SECRET_MAX_BYTES,
        });
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| SecretError::InvalidJson)?;
    let text = trim_one_newline(text);
    let secret: SecretFile = serde_json::from_str(text).map_err(|_| SecretError::InvalidJson)?;
    if secret.schema_version != SECRET_SCHEMA_VERSION {
        return Err(SecretError::UnsupportedSchema {
            found: secret.schema_version,
        });
    }
    if secret.api_key.trim().is_empty() {
        return Err(SecretError::EmptyKey);
    }
    Ok(Some(secret.api_key))
}

fn write_temp(temp: &Path, bytes: &[u8]) -> Result<(), SecretError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Mode at creation, not set_permissions afterwards: there is no
        // 0644 window between creation and the chmod (ADR §2-1-2).
        options.mode(0o600);
    }
    let mut file = options
        .open(temp)
        .map_err(|e| io_err("create temp", temp, e))?;
    file.write_all(bytes)
        .map_err(|e| io_err("write temp", temp, e))?;
    file.write_all(b"\n")
        .map_err(|e| io_err("write temp", temp, e))?;
    file.sync_all().map_err(|e| io_err("sync temp", temp, e))?;
    Ok(())
}

/// "One trailing newline" is literal (ADR §2-1-6). Accept CRLF from editors.
fn trim_one_newline(text: &str) -> &str {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text)
}

fn io_err(op: &'static str, path: &Path, source: std::io::Error) -> SecretError {
    SecretError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

/// The journal's `sync_dir_best_effort` is private to its module, so the
/// same two-line fallback lives here (ADR §2-1-3 step 5: POSIX only).
#[cfg(unix)]
fn sync_dir_best_effort(dir: &Path) {
    if let Ok(f) = std::fs::File::open(dir) {
        let _ = f.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir_best_effort(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> (tempfile::TempDir, PathBuf) {
        let dir = crate::test_tempdir();
        let path = dir.path().join(SECRET_FILE_NAME);
        (dir, path)
    }

    fn write_secret(path: &Path, body: &str) {
        std::fs::write(path, body).expect("fixture");
    }

    #[test]
    fn spec_f014_file_wins_over_environment() {
        let (_dir, path) = path();
        save_key(&path, "file-key").expect("save");
        let (key, status) = load_key(&path, Some("env-key")).expect("load");
        assert_eq!(key.as_deref(), Some("file-key"));
        assert_eq!(status.source, KeySource::File);
        assert!(status.has_key);
    }

    #[test]
    fn spec_f014_absent_file_falls_back_to_environment() {
        let (_dir, path) = path();
        let (key, status) = load_key(&path, Some("env-key")).expect("load");
        assert_eq!(key.as_deref(), Some("env-key"));
        assert_eq!(status.source, KeySource::Environment);
        assert!(status.has_key);
    }

    #[test]
    fn spec_f014_no_file_and_no_environment_is_none() {
        let (_dir, path) = path();
        let (key, status) = load_key(&path, None).expect("load");
        assert!(key.is_none());
        assert_eq!(status.source, KeySource::None);
        assert!(!status.has_key);
    }

    #[test]
    fn spec_f014_empty_environment_is_not_a_key() {
        let (_dir, path) = path();
        let (key, status) = load_key(&path, Some("  ")).expect("load");
        assert!(key.is_none());
        assert_eq!(status.source, KeySource::None);
    }

    #[test]
    fn spec_f014_corrupt_file_is_an_error_not_an_environment_fallback() {
        let (_dir, path) = path();
        write_secret(&path, r#"{"schema_version":1,"api_key":"#);
        let result = load_key(&path, Some("env-secret"));
        let err = result.expect_err("corrupt file must not fall back");
        assert!(matches!(err, SecretError::InvalidJson), "got {err:?}");
        assert!(
            !err.to_string().contains("env-secret"),
            "the environment key leaked: {err}"
        );
    }

    #[test]
    fn spec_f014_unknown_field_is_rejected() {
        let (_dir, path) = path();
        write_secret(&path, r#"{"schema_version":1,"api_key":"k","extra":true}"#);
        assert!(matches!(
            load_key(&path, None),
            Err(SecretError::InvalidJson)
        ));
    }

    #[test]
    fn spec_f014_wrong_schema_version_is_rejected() {
        let (_dir, path) = path();
        write_secret(&path, r#"{"schema_version":2,"api_key":"k"}"#);
        assert!(matches!(
            load_key(&path, None),
            Err(SecretError::UnsupportedSchema { found: 2 })
        ));
    }

    #[test]
    fn spec_f014_empty_key_is_rejected() {
        let (_dir, path) = path();
        write_secret(&path, r#"{"schema_version":1,"api_key":"   "}"#);
        assert!(matches!(load_key(&path, None), Err(SecretError::EmptyKey)));
    }

    #[test]
    fn spec_f014_oversized_file_is_rejected_before_reading() {
        let (_dir, path) = path();
        std::fs::write(&path, vec![b'x'; (SECRET_MAX_BYTES + 1) as usize]).expect("fixture");
        let err = load_key(&path, Some("env-secret")).expect_err("too large");
        assert!(
            matches!(
                err,
                SecretError::TooLarge {
                    max: SECRET_MAX_BYTES
                }
            ),
            "got {err:?}"
        );
        assert!(!err.to_string().contains("env-secret"));
    }

    #[test]
    fn spec_f014_write_then_read_round_trips() {
        let (_dir, path) = path();
        save_key(&path, "my-api-key").expect("save");
        let (key, status) = load_key(&path, None).expect("load");
        assert_eq!(key.as_deref(), Some("my-api-key"));
        assert_eq!(status.source, KeySource::File);
    }

    #[test]
    fn spec_f014_save_trims_one_trailing_newline() {
        let (_dir, path) = path();
        save_key(&path, "my-api-key\n").expect("save");
        let (key, _) = load_key(&path, None).expect("load");
        assert_eq!(key.as_deref(), Some("my-api-key"));
    }

    #[test]
    fn spec_f014_write_leaves_no_temp_behind() {
        let (_dir, path) = path();
        save_key(&path, "my-api-key").expect("save");
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().expect("parent"))
            .expect("read_dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temporaries left: {leftovers:?}");
    }

    #[test]
    fn spec_f014_stale_temp_does_not_block_the_next_save() {
        // The whole reason for the unique-name protocol: `write_atomic`'s
        // fixed `.part` is never deleted after a crash, so it would refuse
        // every later save. A foreign `.<name>.<uuid>.tmp` must not.
        let (dir, path) = path();
        let stale = dir.path().join(format!(".{SECRET_FILE_NAME}.crashed.tmp"));
        std::fs::write(&stale, b"half-written").expect("stale temp");
        save_key(&path, "after-crash").expect("save must not be blocked");
        let (key, status) = load_key(&path, None).expect("load");
        assert_eq!(key.as_deref(), Some("after-crash"));
        assert_eq!(status.source, KeySource::File);
        assert!(stale.exists(), "a foreign temp is not ours to delete");
    }

    #[test]
    fn spec_f014_symlinked_target_is_refused() {
        let (dir, path) = path();
        let real = dir.path().join("elsewhere.json");
        std::fs::write(&real, b"{}").expect("fixture");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real, &path).expect("symlink");
            assert!(matches!(save_key(&path, "k"), Err(SecretError::Symlink)));
            assert_eq!(std::fs::read(&real).expect("read"), b"{}");
        }
        #[cfg(not(unix))]
        {
            // Creating symlinks on Windows needs privileges; the refusal is
            // still compiled in `save_key`, this test only covers unix.
            let _ = (dir, path, real);
        }
    }

    #[cfg(unix)]
    #[test]
    fn spec_f014_unix_secret_is_0600_at_creation() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path) = path();
        save_key(&path, "my-api-key").expect("save");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "secret file mode was {mode:o}");
    }
}
