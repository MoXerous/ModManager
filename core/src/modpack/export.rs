//! Exporting an installed instance as a Modrinth `.mrpack` (F-081, F-082).
//!
//! Two halves, and the split is the point:
//!
//! * **Planning** resolves the instance's pinned content (`instance.lock.json`)
//!   through the provider and collects the local files the user chose to ship,
//!   reading both and writing nothing. The preview is the deliverable, exactly
//!   like every other mutation in this project (AGENTS.md §5.4).
//! * **Writing** turns a plan into a zip: the generated `modrinth.index.json`
//!   first, then the overrides. The archive is built as a temporary file next
//!   to the destination and renamed into place only after it is complete, so a
//!   crash never leaves a half pack where a pack is expected.
//!
//! The pack only references files by URL and hash: no mod jar is copied into
//! the index, and nothing from the instance is uploaded anywhere. Overrides are
//! the author's local configuration, and they pass the same exclusion policy
//! the profile export uses (F-156): logs, `.env` files, keys, certificates and
//! anything whose name looks like a secret never enter a pack.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha512};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::instances::lock::{read_lock, LockEntry, LockError, LOCK_FILE_NAME};
use crate::providers::{Project, Provider, Version};
use crate::safety::path::safe_member_name;

/// The folder each exportable Modrinth project type lands in.
///
/// `modpack` is absent on purpose: a pack nested in a pack is not an instance
/// file, and the old exporter skipped it too.
pub fn folder_for_project_type(project_type: &str) -> Option<&'static str> {
    match project_type {
        "mod" => Some("mods"),
        "resourcepack" => Some("resourcepacks"),
        "shader" => Some("shaderpacks"),
        "datapack" => Some("datapacks"),
        "plugin" => Some("plugins"),
        _ => None,
    }
}

/// The dependency key the format uses for a loader name.
///
/// Unknown loaders return `None`: the pack still carries `minecraft`, and
/// guessing a loader key would make a pack that claims a loader it does not
/// have.
pub fn loader_key(loader: &str) -> Option<&'static str> {
    match loader.trim().to_lowercase().as_str() {
        "fabric" => Some("fabric-loader"),
        "quilt" => Some("quilt-loader"),
        "forge" => Some("forge"),
        "neoforge" => Some("neoforge"),
        _ => None,
    }
}

/// Why one locked project is not part of the exported pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// Lock entries from a provider whose files cannot go into a Modrinth pack
    /// (CurseForge arrives with its own zip format in F-083).
    UnsupportedProvider,
    /// A hand-edited lock entry with no pinned version.
    MissingVersionId,
    /// The provider could not answer for this version.
    VersionUnavailable,
    /// The provider could not answer for this project.
    ProjectUnavailable,
    /// The version has no downloadable file at all.
    NoFile,
    /// The chosen file has no URL or no usable host.
    InvalidUrl,
    /// The pack format requires a positive size.
    MissingSize,
    /// `.mrpack` requires both hashes; one is missing or malformed.
    InvalidHashes,
    /// The project type has no instance folder (a nested modpack, a tool, …).
    UnsupportedType,
    /// Another file already claimed this destination.
    DuplicatePath,
    /// The provider's file name is not a safe instance-relative path.
    UnsafeName,
}

impl SkipReason {
    /// Stable identifier the UI translates (`export.skip.<id>`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedProvider => "unsupported_provider",
            Self::MissingVersionId => "missing_version_id",
            Self::VersionUnavailable => "version_unavailable",
            Self::ProjectUnavailable => "project_unavailable",
            Self::NoFile => "no_file",
            Self::InvalidUrl => "invalid_url",
            Self::MissingSize => "missing_size",
            Self::InvalidHashes => "invalid_hashes",
            Self::UnsupportedType => "unsupported_type",
            Self::DuplicatePath => "duplicate_path",
            Self::UnsafeName => "unsafe_name",
        }
    }
}

/// One project the pack will reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportMod {
    pub project_id: String,
    pub version_id: String,
    pub title: String,
    pub project_type: String,
    pub folder: &'static str,
    pub file_name: String,
    pub url: String,
    pub size: u64,
    pub sha1: String,
    pub sha512: String,
}

impl ExportMod {
    /// `folder/file` as it appears in `files[].path`.
    pub fn destination(&self) -> String {
        format!("{}/{}", self.folder, self.file_name)
    }
}

/// One locked project that is not exported, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSkip {
    pub project_id: String,
    pub version_id: String,
    pub reason: SkipReason,
}

/// Why a local path is not copied into the archive (F-156).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exclusion {
    /// VCS directories, logs, crash reports and the app's own transient trees.
    Transient,
    /// `.env` files and their variants.
    Environment,
    /// Keys, certificates, private files, and secret-looking names.
    Secret,
    /// A symlink: following it could copy files from outside the instance.
    Symlink,
    /// The same archive path was already claimed by another folder.
    Duplicate,
    /// The name cannot be represented safely inside a zip.
    UnsafeName,
}

impl Exclusion {
    /// Stable identifier the UI translates (`export.excluded.<id>`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Environment => "environment",
            Self::Secret => "secret",
            Self::Symlink => "symlink",
            Self::Duplicate => "duplicate",
            Self::UnsafeName => "unsafe_name",
        }
    }
}

/// One local file the pack carries under `overrides/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideFile {
    /// Archive member name, always `overrides/<instance-relative path>`.
    pub archive_path: String,
    pub source: PathBuf,
    pub size: u64,
}

/// One local path that was left out of the pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcludedEntry {
    /// Instance-relative path with forward slashes.
    pub path: String,
    pub reason: Exclusion,
}

/// What the user asked to export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSettings {
    pub name: String,
    /// The pack's own version id. The UI pre-fills a timestamp; it is not
    /// derived here because a clock is not this module's business.
    pub version_id: String,
    pub summary: Option<String>,
    pub game_version: String,
    /// The instance's loader name (`fabric`, `quilt`, `forge`, `neoforge`).
    pub loader: String,
    /// Instance-relative folders copied under `overrides/`.
    pub override_folders: Vec<String>,
}

/// The reviewed answer: everything the archive will contain.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportPlan {
    pub name: String,
    pub version_id: String,
    pub summary: Option<String>,
    pub game_version: String,
    pub loader: String,
    pub files: Vec<ExportMod>,
    pub overrides: Vec<OverrideFile>,
    pub skipped: Vec<ExportSkip>,
    pub excluded: Vec<ExcludedEntry>,
    /// SHA-512 of the lock file the plan was built from. The IPC write command
    /// compares it against the lock on disk before writing, so a pack is never
    /// built from a preview of an instance that has changed since.
    pub lock_sha512: String,
}

impl ExportPlan {
    pub fn total_override_bytes(&self) -> u64 {
        self.overrides.iter().map(|o| o.size).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.overrides.is_empty()
    }
}

/// The outcome of writing a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSummary {
    pub output: PathBuf,
    pub file_count: usize,
    pub override_count: usize,
    pub total_size: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("the instance has no {LOCK_FILE_NAME}; install content first")]
    MissingLock,
    #[error("invalid instance lock: {0}")]
    Lock(#[from] LockError),
    #[error("the pack name is empty")]
    EmptyName,
    #[error("the pack version id is empty")]
    EmptyVersionId,
    #[error("the pack has no game version")]
    EmptyGameVersion,
    #[error("override folder {0:?} is not a safe instance-relative directory")]
    UnsafeFolder(String),
    #[error("override folder {0:?} is not an existing directory")]
    NotAFolder(String),
    #[error("the pack is empty: no files were resolved and no overrides were selected")]
    Empty,
    #[error("the destination {0} is a directory")]
    DestinationIsDir(PathBuf),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("the pack archive could not be written: {0}")]
    Zip(String),
}

/// Reads the lock, resolves every entry through the provider, and collects the
/// selected override folders. Reads only: nothing is written and no directory
/// is created.
pub async fn plan_export(
    provider: &dyn Provider,
    instance_root: &Path,
    settings: &ExportSettings,
) -> Result<ExportPlan, ExportError> {
    validate_settings(settings)?;

    let lock_path = instance_root.join(LOCK_FILE_NAME);
    if !lock_path.is_file() {
        return Err(ExportError::MissingLock);
    }
    let lock_sha512 = file_sha512(&lock_path)?;
    let lock = read_lock(&lock_path)?;

    let (files, skipped) = resolve_mods(provider, &lock.entries).await;
    let (overrides, excluded) = collect_overrides(instance_root, &settings.override_folders)?;

    let plan = ExportPlan {
        name: settings.name.trim().to_string(),
        version_id: settings.version_id.trim().to_string(),
        summary: settings
            .summary
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        game_version: settings.game_version.trim().to_string(),
        loader: settings.loader.trim().to_lowercase(),
        files,
        overrides,
        skipped,
        excluded,
        lock_sha512,
    };
    if plan.is_empty() {
        return Err(ExportError::Empty);
    }
    Ok(plan)
}

fn validate_settings(settings: &ExportSettings) -> Result<(), ExportError> {
    if settings.name.trim().is_empty() {
        return Err(ExportError::EmptyName);
    }
    if settings.version_id.trim().is_empty() {
        return Err(ExportError::EmptyVersionId);
    }
    if settings.game_version.trim().is_empty() {
        return Err(ExportError::EmptyGameVersion);
    }
    Ok(())
}

/// Resolves every locked modrinth entry, skipping (and reporting) those the
/// format cannot carry. Sequential on purpose: a pack export is a background
/// operation, and the provider already owns retry and rate-limit answers.
pub async fn resolve_mods(
    provider: &dyn Provider,
    entries: &[LockEntry],
) -> (Vec<ExportMod>, Vec<ExportSkip>) {
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for entry in entries {
        let skip = |reason: SkipReason| ExportSkip {
            project_id: entry.project_id.clone(),
            version_id: entry.version_id.clone(),
            reason,
        };
        if !entry.provider.eq_ignore_ascii_case("modrinth") {
            skipped.push(skip(SkipReason::UnsupportedProvider));
            continue;
        }
        if entry.version_id.trim().is_empty() {
            skipped.push(skip(SkipReason::MissingVersionId));
            continue;
        }
        let version = match provider.get_version(&entry.version_id).await {
            Ok(v) => v,
            Err(_) => {
                skipped.push(skip(SkipReason::VersionUnavailable));
                continue;
            }
        };
        // The lock's project id is the durable identity; a lock written by an
        // older schema may carry none, and the version still knows it.
        let project_id = if entry.project_id.trim().is_empty() {
            version.project_id.clone()
        } else {
            entry.project_id.clone()
        };
        let project = match provider.get_project(&project_id).await {
            Ok(p) => p,
            Err(_) => {
                skipped.push(skip(SkipReason::ProjectUnavailable));
                continue;
            }
        };
        match export_mod_from(&project, &version, &mut seen) {
            Ok(module) => files.push(module),
            Err(reason) => skipped.push(ExportSkip {
                project_id,
                version_id: entry.version_id.clone(),
                reason,
            }),
        }
    }

    (files, skipped)
}

/// Decides whether one resolved project becomes an index entry (F-081).
///
/// Pure: the provider calls are behind it, so every rejection rule is testable
/// without a network. `seen` is keyed on the lowercased destination, because on
/// Windows two files whose names differ only by case are one file.
pub fn export_mod_from(
    project: &Project,
    version: &Version,
    seen: &mut BTreeSet<String>,
) -> Result<ExportMod, SkipReason> {
    let file = version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
        .ok_or(SkipReason::NoFile)?;
    if !is_https_url(&file.url) {
        return Err(SkipReason::InvalidUrl);
    }
    if file.size == 0 {
        return Err(SkipReason::MissingSize);
    }
    let sha1 = file.sha1.as_deref().unwrap_or("");
    let sha512 = file.sha512.as_deref().unwrap_or("");
    if !is_hex(sha1, 40) || !is_hex(sha512, 128) {
        return Err(SkipReason::InvalidHashes);
    }
    let folder =
        folder_for_project_type(&project.project_type).ok_or(SkipReason::UnsupportedType)?;
    // The provider names the file; the format wants one path component. A
    // provider value carrying `a/b.jar` keeps its last component, exactly like
    // the old exporter, and a value that cannot be a safe member is refused
    // rather than normalised into something else.
    let raw_name = file.filename.rsplit(['/', '\\']).next().unwrap_or("");
    if raw_name.is_empty() {
        return Err(SkipReason::UnsafeName);
    }
    let destination =
        safe_member_name(&format!("{folder}/{raw_name}")).map_err(|_| SkipReason::UnsafeName)?;
    // A trailing slash would name a directory, not a file, in `files[].path`.
    if destination.ends_with('/') {
        return Err(SkipReason::UnsafeName);
    }
    if seen.contains(&destination.to_lowercase()) {
        return Err(SkipReason::DuplicatePath);
    }
    seen.insert(destination.to_lowercase());

    Ok(ExportMod {
        project_id: project.id.clone(),
        version_id: version.id.clone(),
        title: if project.title.is_empty() {
            project.id.clone()
        } else {
            project.title.clone()
        },
        project_type: project.project_type.clone(),
        folder,
        file_name: raw_name.to_string(),
        url: file.url.clone(),
        size: file.size,
        sha1: sha1.to_lowercase(),
        sha512: sha512.to_lowercase(),
    })
}

/// A download URL this exporter is willing to write: `https` with a host.
///
/// The old exporter also accepted `http`. This one does not: every provider
/// this application reads from serves `https`, so a plain-`http` URL is either
/// a stale lock or a hand-edited one, and writing it into a pack would invite a
/// silent downgrade for everyone who later installs that pack.
pub fn is_https_url(url: &str) -> bool {
    let rest = match url.strip_prefix("https://") {
        Some(rest) => rest,
        None => return false,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains(char::is_whitespace) {
        return false;
    }
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or("");
    !host.is_empty() && !host.starts_with('.')
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn file_sha512(path: &Path) -> Result<String, ExportError> {
    let mut file = std::fs::File::open(path).map_err(|source| ExportError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha512::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| ExportError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

// --- override collection ---------------------------------------------------

/// Directories whose whole subtree is never exported (F-156).
pub const EXCLUDED_DIR_NAMES: [&str; 13] = [
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    "logs",
    "crash-reports",
    "crash_reports",
    ".modrinth_bulk_downloader_backup",
    ".modrinth_bulk_downloader_profile_backups",
    ".modrinth_bulk_downloader_removed_content",
    ".modrinth_bulk_downloader_restore_safety",
    ".modrinth_bulk_downloader_staging",
    // This application's own staging tree: its scratch files are never pack
    // content, and a crash there must not leak a half download into a pack.
    ".mm-staging",
];

/// File names that are never exported, whatever their folder (F-156).
pub const EXCLUDED_FILE_NAMES: [&str; 6] = [
    ".env",
    ".env.local",
    ".env.development",
    ".env.production",
    ".env.test",
    ".modrinth_bulk_downloader_mrpack_install.json",
];

/// Suffixes that mark logs, keys, certificates and private material (F-156).
pub const EXCLUDED_SUFFIXES: [&str; 13] = [
    ".cer",
    ".crt",
    ".der",
    ".env",
    ".gpg",
    ".jks",
    ".key",
    ".keystore",
    ".log",
    ".p12",
    ".pem",
    ".pfx",
    ".pgp",
];

/// Name tokens that mark a secret (F-156). Matched against alphabetic tokens,
/// so `curseforge.secret.json` is caught by `secret`.
pub const SENSITIVE_NAME_TOKENS: [&str; 12] = [
    "account",
    "activation",
    "auth",
    "credential",
    "credentials",
    "license",
    "password",
    "passwd",
    "private",
    "secret",
    "session",
    "token",
];

/// Why `relative` (a path below the selected folder) is excluded, if it is.
///
/// `is_dir` decides whether the directory rules apply to the last component.
/// Every component is checked, so a file inside `logs/` is excluded even when
/// the walk somehow reached it.
pub fn exclusion_reason(relative: &str, is_dir: bool) -> Option<Exclusion> {
    let parts: Vec<String> = relative
        .split('/')
        .filter(|p| !p.is_empty())
        .map(|p| p.to_lowercase())
        .collect();
    if parts.is_empty() {
        return None;
    }
    if parts
        .iter()
        .any(|p| EXCLUDED_DIR_NAMES.contains(&p.as_str()))
    {
        return Some(Exclusion::Transient);
    }
    if is_dir {
        return None;
    }
    let name = parts.last().expect("non-empty after the guard");
    if EXCLUDED_FILE_NAMES.contains(&name.as_str()) {
        return Some(Exclusion::Environment);
    }
    if EXCLUDED_SUFFIXES
        .iter()
        .any(|suffix| name.ends_with(suffix))
    {
        return Some(Exclusion::Secret);
    }
    let tokens: BTreeSet<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.iter().any(|t| SENSITIVE_NAME_TOKENS.contains(t)) {
        return Some(Exclusion::Secret);
    }
    None
}

/// Collects the selected folders into archive members, sorted and bounded.
pub fn collect_overrides(
    instance_root: &Path,
    folders: &[String],
) -> Result<(Vec<OverrideFile>, Vec<ExcludedEntry>), ExportError> {
    let mut files = Vec::new();
    let mut excluded = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for folder in folders {
        let normalized =
            safe_member_name(folder).map_err(|_| ExportError::UnsafeFolder(folder.clone()))?;
        let normalized = normalized
            .strip_suffix('/')
            .unwrap_or(&normalized)
            .to_string();
        if normalized.is_empty() {
            return Err(ExportError::UnsafeFolder(folder.clone()));
        }
        let root = instance_root.join(&normalized);
        let meta = std::fs::symlink_metadata(&root)
            .map_err(|_| ExportError::NotAFolder(folder.clone()))?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(ExportError::NotAFolder(folder.clone()));
        }
        walk_override_dir(&root, &normalized, 0, &mut files, &mut excluded, &mut seen)?;
    }

    Ok((files, excluded))
}

fn walk_override_dir(
    dir: &Path,
    relative: &str,
    depth: usize,
    files: &mut Vec<OverrideFile>,
    excluded: &mut Vec<ExcludedEntry>,
    seen: &mut BTreeSet<String>,
) -> Result<(), ExportError> {
    if depth > 64 {
        excluded.push(ExcludedEntry {
            path: relative.to_string(),
            reason: Exclusion::UnsafeName,
        });
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|source| ExportError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut children: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    children.sort_by_key(|e| e.file_name());

    for child in children {
        let name = child.file_name();
        let Some(name) = name.to_str() else {
            excluded.push(ExcludedEntry {
                path: join_slash(relative, &name.to_string_lossy()),
                reason: Exclusion::UnsafeName,
            });
            continue;
        };
        // A backslash is a legal filename character on Unix but a separator on
        // Windows and in zip readers: one archive path that means two things is
        // never written. The whole component check also catches reserved
        // device names (`aux`, `con`, …), which a pack reader on Windows would
        // refuse anyway.
        if name.contains('\\') || safe_member_name(name).is_err() {
            excluded.push(ExcludedEntry {
                path: join_slash(relative, name),
                reason: Exclusion::UnsafeName,
            });
            continue;
        }
        let child_rel = join_slash(relative, name);
        let path = child.path();
        // `symlink_metadata` never follows the link, so a directory symlink is
        // reported as a link and the walk stops there instead of leaving the
        // instance (and copying whatever it points at).
        let meta = std::fs::symlink_metadata(&path).map_err(|source| ExportError::Io {
            path: path.clone(),
            source,
        })?;
        if meta.file_type().is_symlink() {
            excluded.push(ExcludedEntry {
                path: child_rel,
                reason: Exclusion::Symlink,
            });
            continue;
        }
        if meta.is_dir() {
            if let Some(reason) = exclusion_reason(&child_rel, true) {
                excluded.push(ExcludedEntry {
                    path: child_rel,
                    reason,
                });
                continue;
            }
            walk_override_dir(&path, &child_rel, depth + 1, files, excluded, seen)?;
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        if let Some(reason) = exclusion_reason(&child_rel, false) {
            excluded.push(ExcludedEntry {
                path: child_rel,
                reason,
            });
            continue;
        }
        let archive_path = match safe_member_name(&format!("overrides/{child_rel}")) {
            Ok(p) => p,
            Err(_) => {
                excluded.push(ExcludedEntry {
                    path: child_rel,
                    reason: Exclusion::UnsafeName,
                });
                continue;
            }
        };
        let key = archive_path.to_lowercase();
        if !seen.insert(key) {
            excluded.push(ExcludedEntry {
                path: child_rel,
                reason: Exclusion::Duplicate,
            });
            continue;
        }
        files.push(OverrideFile {
            archive_path,
            source: path,
            size: meta.len(),
        });
    }
    Ok(())
}

fn join_slash(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

// --- index and archive -----------------------------------------------------

/// The generated index, as a JSON value. Validates the plan's own output so a
/// pack this application writes is a pack this application can read back.
pub fn index_json(plan: &ExportPlan) -> Value {
    let mut dependencies = Map::new();
    dependencies.insert(
        "minecraft".to_string(),
        Value::String(plan.game_version.clone()),
    );
    if let Some(key) = loader_key(&plan.loader) {
        dependencies.insert(key.to_string(), Value::String("*".to_string()));
    }

    let files: Vec<Value> = plan
        .files
        .iter()
        .map(|file| {
            json!({
                "path": file.destination(),
                "hashes": { "sha1": file.sha1, "sha512": file.sha512 },
                "downloads": [file.url],
                "fileSize": file.size,
            })
        })
        .collect();

    let mut index = Map::new();
    index.insert("formatVersion".to_string(), json!(1));
    index.insert("game".to_string(), json!("minecraft"));
    index.insert("versionId".to_string(), json!(plan.version_id));
    index.insert("name".to_string(), json!(plan.name));
    if let Some(summary) = &plan.summary {
        index.insert("summary".to_string(), json!(summary));
    }
    index.insert("files".to_string(), Value::Array(files));
    index.insert("dependencies".to_string(), Value::Object(dependencies));
    Value::Object(index)
}

/// Writes the plan to `output` as a `.mrpack`.
///
/// The archive is written to a fresh sibling file and renamed over `output`
/// only after `ZipWriter::finish` succeeded. A failure removes the temporary
/// file, so the destination is either the previous file or the complete pack.
pub fn write_export_zip(plan: &ExportPlan, output: &Path) -> Result<ExportSummary, ExportError> {
    if plan.is_empty() {
        return Err(ExportError::Empty);
    }
    if output.is_dir() {
        return Err(ExportError::DestinationIsDir(output.to_path_buf()));
    }
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| ExportError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }

    let index = serde_json::to_vec_pretty(&index_json(plan))
        .map_err(|e| ExportError::Zip(e.to_string()))?;
    let (temp, temp_file) = unique_temp(output)?;

    let result = (|| -> Result<ExportSummary, ExportError> {
        let mut zip = ZipWriter::new(temp_file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file(super::mrpack::INDEX_ENTRY, options)
            .map_err(|e| ExportError::Zip(e.to_string()))?;
        zip.write_all(&index)
            .map_err(|e| ExportError::Zip(e.to_string()))?;
        let mut total_size = index.len() as u64;
        for override_file in &plan.overrides {
            let mut source =
                std::fs::File::open(&override_file.source).map_err(|source| ExportError::Io {
                    path: override_file.source.clone(),
                    source,
                })?;
            zip.start_file(&override_file.archive_path, options)
                .map_err(|e| ExportError::Zip(e.to_string()))?;
            let copied = std::io::copy(&mut source, &mut zip)
                .map_err(|e| ExportError::Zip(e.to_string()))?;
            total_size += copied;
        }
        zip.finish().map_err(|e| ExportError::Zip(e.to_string()))?;
        std::fs::rename(&temp, output).map_err(|source| ExportError::Io {
            path: output.to_path_buf(),
            source,
        })?;
        Ok(ExportSummary {
            output: output.to_path_buf(),
            file_count: plan.files.len(),
            override_count: plan.overrides.len(),
            total_size,
        })
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// A `.part` sibling that did not exist a moment ago; `create_new` is the
/// reservation, so two exports to one path cannot write the same temp file.
/// The open handle comes back with the path: reopening it would be a second
/// chance for something else to claim the name.
fn unique_temp(output: &Path) -> Result<(PathBuf, std::fs::File), ExportError> {
    let parent = output.parent().unwrap_or_else(|| Path::new(""));
    let name = output
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("export.mrpack");
    for attempt in 0..1000u32 {
        let candidate = parent.join(format!(".{name}.{}.{}.part", std::process::id(), attempt));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(ExportError::Io {
                    path: candidate,
                    source,
                })
            }
        }
    }
    Err(ExportError::Zip(
        "could not reserve a temporary export file".to_string(),
    ))
}
