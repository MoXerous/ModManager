//! F-101: `profile.json`, the portable metadata mirror of an instance row.
//!
//! SQLite is the source of truth; this file is a mirror that lets a folder be
//! recognised when it travels to another machine. It is written only when we
//! create the folder (`InstanceSource::Managed`) — a folder the user linked is
//! never written to, which is what F-103 promises.
//!
//! The read side is deliberately tolerant, unlike the operation journal: this
//! file is meant to be portable and edited by other tools, so unknown keys are
//! ignored and missing optional fields fall back. The journal is machine-private
//! and therefore `deny_unknown_fields`. Different threat model, different rule.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::db::instances::InstanceSource;
use crate::safety::atomic::{write_atomic, AtomicError};

/// Bumped whenever the shape changes; a newer value is refused, never guessed.
pub const PROFILE_SCHEMA: &str = "modmanager.profile.v1";
pub const PROFILE_FILE_NAME: &str = "profile.json";

/// A 4 MiB metadata file is already absurd; the journal uses the same cap.
pub const MAX_PROFILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum MetadataError {
    #[error(transparent)]
    Atomic(#[from] AtomicError),
    #[error("profile.json is not valid JSON: {0}")]
    Parse(String),
    #[error("profile.json schema {found:?} is newer than supported {supported:?}")]
    NewerSchema { found: String, supported: String },
    #[error("profile.json is missing the required field {0:?}")]
    MissingField(&'static str),
    #[error("profile.json contains a NUL byte")]
    NulByte,
    #[error("profile.json is {size} bytes, over the {max} byte cap")]
    TooLarge { size: u64, max: u64 },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMetadata {
    pub schema: String,
    pub id: String,
    pub name: String,
    /// `None` is written as `null`, not as the storage sentinel: this file is
    /// read by humans and by other tools, where "absent" is the honest word.
    #[serde(default)]
    pub minecraft_version: Option<String>,
    #[serde(default)]
    pub loader: Option<String>,
    pub source: String,
    pub instance_root: String,
    pub minecraft_root: String,
    pub created_at: i64,
}

impl ProfileMetadata {
    pub fn typed_source(&self) -> Result<InstanceSource, MetadataError> {
        InstanceSource::parse(&self.source)
            .map_err(|_| MetadataError::Parse(format!("unknown instance source {:?}", self.source)))
    }

    pub fn roots(&self) -> (PathBuf, PathBuf) {
        (
            PathBuf::from(&self.instance_root),
            PathBuf::from(&self.minecraft_root),
        )
    }
}

/// `<instance_root>/profile.json`.
pub fn mirror_path(instance_root: &Path) -> PathBuf {
    instance_root.join(PROFILE_FILE_NAME)
}

/// Serializes to the mirror. Callers must already have the database row: this
/// is the second half of a write, never the first.
pub fn write_mirror(instance_root: &Path, meta: &ProfileMetadata) -> Result<(), MetadataError> {
    let mut bytes =
        serde_json::to_vec_pretty(meta).map_err(|e| MetadataError::Parse(e.to_string()))?;
    bytes.push(b'\n');
    write_atomic(&mirror_path(instance_root), &mut bytes.as_slice())?;
    Ok(())
}

pub fn read_mirror(path: &Path) -> Result<ProfileMetadata, MetadataError> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_PROFILE_BYTES {
        return Err(MetadataError::TooLarge {
            size: meta.len(),
            max: MAX_PROFILE_BYTES,
        });
    }
    let bytes = std::fs::read(path)?;
    if bytes.contains(&0) {
        return Err(MetadataError::NulByte);
    }
    let meta: ProfileMetadata =
        serde_json::from_slice(&bytes).map_err(|e| MetadataError::Parse(e.to_string()))?;
    if meta.schema != PROFILE_SCHEMA {
        return Err(MetadataError::NewerSchema {
            found: meta.schema,
            supported: PROFILE_SCHEMA.to_string(),
        });
    }
    if meta.id.is_empty() {
        return Err(MetadataError::MissingField("id"));
    }
    if meta.name.is_empty() {
        return Err(MetadataError::MissingField("name"));
    }
    Ok(meta)
}

/// Builds the mirror content from a stored row. `minecraft_root` is filled in
/// when the row left it blank, so the file never contains an empty path.
pub fn mirror_of(row: &crate::db::instances::InstanceRow) -> ProfileMetadata {
    let minecraft_root = if row.minecraft_root.as_os_str().is_empty() {
        row.path.clone()
    } else {
        row.minecraft_root.clone()
    };
    ProfileMetadata {
        schema: PROFILE_SCHEMA.to_string(),
        id: row.id.clone(),
        name: row.name.clone(),
        minecraft_version: row.minecraft_version.clone(),
        loader: row.loader.clone(),
        source: row.source.as_str().to_string(),
        instance_root: row.path.to_string_lossy().into_owned(),
        minecraft_root: minecraft_root.to_string_lossy().into_owned(),
        created_at: row.created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::instances::UNKNOWN;

    fn sample(root: &Path) -> ProfileMetadata {
        ProfileMetadata {
            schema: PROFILE_SCHEMA.to_string(),
            id: "i1".to_string(),
            name: "Survival".to_string(),
            minecraft_version: Some("1.21.1".to_string()),
            loader: Some("fabric".to_string()),
            source: "managed".to_string(),
            instance_root: root.to_string_lossy().into_owned(),
            minecraft_root: root.to_string_lossy().into_owned(),
            created_at: 7,
        }
    }

    fn temp() -> tempfile::TempDir {
        crate::test_tempdir()
    }

    #[test]
    fn spec_f101_profile_json_round_trips() {
        let dir = temp();
        let root = dir.path().join("Survival");
        std::fs::create_dir_all(&root).expect("mkdir");
        let meta = sample(&root);

        write_mirror(&root, &meta).expect("write");
        let back = read_mirror(&mirror_path(&root)).expect("read");
        assert_eq!(back, meta);
        assert_eq!(
            back.typed_source().expect("typed source"),
            InstanceSource::Managed
        );
    }

    #[test]
    fn spec_f101_profile_json_records_unknown_as_null_not_sentinel() {
        let dir = temp();
        let root = dir.path().join("Undetected");
        std::fs::create_dir_all(&root).expect("mkdir");
        let mut meta = sample(&root);
        meta.minecraft_version = None;
        meta.loader = None;

        write_mirror(&root, &meta).expect("write");
        let raw = std::fs::read_to_string(mirror_path(&root)).expect("read raw");
        assert!(
            raw.contains("\"loader\": null"),
            "expected null, got: {raw}"
        );
        assert!(
            !raw.contains(UNKNOWN),
            "the storage sentinel must never reach the portable file"
        );
        assert_eq!(read_mirror(&mirror_path(&root)).expect("read").loader, None);
    }

    #[test]
    fn spec_f101_profile_json_ignores_unknown_keys_and_missing_optionals() {
        let dir = temp();
        let root = dir.path().join("Forward");
        std::fs::create_dir_all(&root).expect("mkdir");
        let raw = r#"{
            "schema": "modmanager.profile.v1",
            "id": "i1",
            "name": "Survival",
            "source": "manual",
            "instance_root": "C:/x",
            "minecraft_root": "C:/x",
            "created_at": 1,
            "something_a_newer_tool_added": {"nested": [1, 2, 3]}
        }"#;
        std::fs::write(mirror_path(&root), raw).expect("write");
        let meta = read_mirror(&mirror_path(&root)).expect("tolerant read");
        assert_eq!(meta.minecraft_version, None, "absent optional stays absent");
        assert_eq!(meta.loader, None);
        assert_eq!(meta.typed_source().expect("typed"), InstanceSource::Manual);
    }

    #[test]
    fn spec_f101_profile_json_refuses_a_newer_schema() {
        let dir = temp();
        let root = dir.path().join("Newer");
        std::fs::create_dir_all(&root).expect("mkdir");
        let raw = r#"{"schema":"modmanager.profile.v9","id":"i1","name":"n","source":"managed",
            "instance_root":"C:/x","minecraft_root":"C:/x","created_at":1}"#;
        std::fs::write(mirror_path(&root), raw).expect("write");
        let err = read_mirror(&mirror_path(&root)).expect_err("newer refused");
        assert!(
            matches!(err, MetadataError::NewerSchema { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn spec_f101_profile_json_rejects_nul_and_oversize() {
        let dir = temp();
        let root = dir.path().join("Bad");
        std::fs::create_dir_all(&root).expect("mkdir");

        // A real NUL byte, not the two characters `\0`.
        let mut nul: Vec<u8> = br#"{"schema":"modmanager.profile.v1","id":"i"#.to_vec();
        nul.push(0);
        nul.extend_from_slice(b"d\",\"name\":\"n\",\"source\":\"managed\",\"instance_root\":\"C:/x\",\"minecraft_root\":\"C:/x\",\"created_at\":1}");
        std::fs::write(mirror_path(&root), nul).expect("write");
        assert!(matches!(
            read_mirror(&mirror_path(&root)).expect_err("nul refused"),
            MetadataError::NulByte
        ));

        std::fs::write(
            mirror_path(&root),
            vec![b'x'; (MAX_PROFILE_BYTES + 1) as usize],
        )
        .expect("write big");
        assert!(matches!(
            read_mirror(&mirror_path(&root)).expect_err("oversize refused"),
            MetadataError::TooLarge { .. }
        ));
    }

    #[test]
    fn spec_f101_profile_json_write_leaves_no_part_file() {
        let dir = temp();
        let root = dir.path().join("Clean");
        std::fs::create_dir_all(&root).expect("mkdir");
        write_mirror(&root, &sample(&root)).expect("write");
        let leftovers: Vec<String> = std::fs::read_dir(&root)
            .expect("ls")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .filter(|n| n != PROFILE_FILE_NAME)
            .collect();
        assert!(leftovers.is_empty(), "unexpected leftovers: {leftovers:?}");
    }

    #[test]
    fn spec_f102_mirror_of_fills_a_blank_minecraft_root() {
        let dir = temp();
        let root = dir.path().join("Blank");
        std::fs::create_dir_all(&root).expect("mkdir");
        let canonical = std::fs::canonicalize(&root).expect("canon");
        let row = crate::db::instances::InstanceRow {
            id: "i1".to_string(),
            name: "Blank".to_string(),
            path: canonical.clone(),
            minecraft_root: PathBuf::new(),
            minecraft_version: Some("1.21.1".to_string()),
            loader: Some("fabric".to_string()),
            source: InstanceSource::Managed,
            created_at: 3,
            scanned_at: None,
        };
        let meta = mirror_of(&row);
        assert_eq!(meta.instance_root, canonical.to_string_lossy());
        assert_eq!(meta.minecraft_root, canonical.to_string_lossy());
    }
}
