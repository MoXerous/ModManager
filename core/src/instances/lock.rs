//! `instance.lock.json`: pinned install state. Newer schemas are
//! refused, never migrated silently (mirrors AGENTS.md §5.10).

/// Current lock schema; writers always emit this.
pub const LOCK_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LockEntry {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub version_id: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub sha1: String,
    /// Recorded alongside `sha1` because that is the only durable record of
    /// what was installed: the download verified both, but a later audit or a
    /// re-verify can only use what is written down. SHA-1 is collision-broken,
    /// which is exactly the property a mod channel must not rely on.
    ///
    /// `default` keeps locks written before this field readable — an absent
    /// value reads as empty, which every consumer already handles for `sha1`.
    #[serde(default)]
    pub sha512: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
struct LockDoc {
    #[serde(default = "schema_one")]
    schema_version: u32,
    #[serde(default)]
    entries: Vec<LockEntry>,
}

fn schema_one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LockFile {
    pub entries: Vec<LockEntry>,
}

pub fn read_lock(path: &std::path::Path) -> Result<LockFile, LockError> {
    let text = std::fs::read_to_string(path).map_err(LockError::Io)?;
    read_lock_str(&text)
}

fn read_lock_str(text: &str) -> Result<LockFile, LockError> {
    let doc: LockDoc = serde_json::from_str(text).map_err(|_| LockError::Invalid)?;
    if doc.schema_version > LOCK_SCHEMA_VERSION {
        return Err(LockError::NewerSchema {
            found: doc.schema_version,
            supported: LOCK_SCHEMA_VERSION,
        });
    }
    Ok(LockFile {
        entries: doc.entries,
    })
}

pub fn write_lock(path: &std::path::Path, lock: &LockFile) -> Result<(), LockError> {
    let doc = LockDoc {
        schema_version: LOCK_SCHEMA_VERSION,
        entries: lock.entries.clone(),
    };
    let text = serde_json::to_string_pretty(&doc).map_err(|_| LockError::Invalid)?;
    std::fs::write(path, text).map_err(LockError::Io)?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("io error")]
    Io(std::io::Error),
    #[error("invalid lock file")]
    Invalid,
    #[error("unsupported lock schema {found}, supported {supported}")]
    NewerSchema { found: u32, supported: u32 },
}
