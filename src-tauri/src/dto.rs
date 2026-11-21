//! أنواع تعبر حدّ IPC. كل نوع هنا يُولَّد له ملف TypeScript في src/bindings
//! عند تشغيل `cargo test --workspace`.
//!
//! DTOs are the anti-corruption layer: core types never cross IPC, so
//! core can change without breaking the committed bindings.

use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub os: String,
    pub arch: String,
}

impl From<mm_core::AppInfo> for AppInfo {
    fn from(info: mm_core::AppInfo) -> Self {
        Self {
            name: info.name.to_owned(),
            version: info.version.to_owned(),
            os: info.os.to_owned(),
            arch: info.arch.to_owned(),
        }
    }
}

/// F-151: the roots this process resolved, and the state of its database.
///
/// Paths cross IPC because the screen's job is to show them; they are not
/// secrets, and the open actions never take a path back from the webview.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppPathsDto {
    pub data: String,
    pub config: String,
    pub cache: String,
    pub logs: String,
    pub db_file: String,
    /// Size of the main database file; WAL pages sit next to it.
    #[ts(type = "number")]
    pub db_bytes: u64,
    pub db_version: u32,
    pub app_version: String,
}

/// رمز الخطأ المرسل للواجهة؛ تترجمه الواجهة إلى نص حسب لغة المستخدم.
/// `message` للتشخيص فقط (سجلّ) ولا يُعرض أبدًا.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ErrorCode {
    Internal,
    InvalidInput,
    /// The archive is not the one the shown plan was built from: it changed on
    /// disk between the preview and the confirm. A malformed request is
    /// `InvalidInput`; this is a file the user must look at again.
    PackChanged,
    /// The instance's pinned content changed after an export plan was shown.
    /// The same shape as `PackChanged`, for the other side of the plan.
    InstanceChanged,
    /// The instance has no `instance.lock.json`: nothing is recorded as
    /// installed, so there is nothing to pin in a pack. Not malformed input,
    /// and not an empty disk either — the user must install content first.
    NoInstanceLock,
    /// The chosen settings produced a pack with no indexed files and no
    /// overrides. Refusing here is the difference between an empty archive and
    /// a pack that silently says it contains nothing.
    EmptyExport,
    NotFound,
    /// 404 من المزوّد.
    ProviderApi,
    /// 401/403 من المزوّد: المفتاح مفقود أو مرفوض، والواجهة توجّه إلى الإعدادات.
    ProviderUnauthorized,
    /// المزوّد لا يقدّم هذه القدرة أصلاً (مثل البحث بالهاش)، وليس خطأً داخليًا.
    Unsupported,
    RateLimited,
    /// `retry_after_seconds` يحمل الأوقات؛ الواجهة لا تحلّل نصًا.
    Timeout,
    Io,
    /// `F-038`: a symlink or NTFS junction sits where the bytes would land.
    /// Not a network failure and not invalid input: the destination tree
    /// holds a link the user must remove (or choose another folder) before
    /// a retry can succeed.
    LinkOrReparse,
    /// مخطط القفل أحدث من المدعوم (§5.10) — لا تُعرض قائمة فارغة.
    NewerSchema,
    /// F-036: the user cancelled the operation. Not an IO failure and not a
    /// failure at all: the batch stopped by request, the files that already
    /// landed stay landed, and the in-flight file keeps its `.part` so a
    /// retry resumes rather than restarting.
    Cancelled,
    /// F-036: an install was started while another one is still registered.
    /// Refused on purpose — two batches sharing one cancel flag would break
    /// both — and the UI tells the user to wait for, or stop, the running one.
    Busy,
    /// F-137: SQLite refused a write because another connection holds the
    /// lock past `busy_timeout`. Not `Busy`, which is the install slot: this
    /// one is a database state the user resolves outside the app.
    DatabaseBusy,
    /// F-138: the database could not be upgraded. The message carries the
    /// stage and the schema versions, never a directory.
    MigrationFailed,
    /// F-114: the backup the user confirmed is not the one the preview showed —
    /// a different instance, or another folder now sits at the same name. Not
    /// `InvalidInput`: the request was well-formed, the disk moved.
    RestoreMismatch,
    /// F-114: the backup's manifest changed after the plan was read (its SHA-256
    /// is the token the confirm carries). Nothing was written.
    BackupChanged,
    /// F-158: the archive the user chose is not this application's export —
    /// no `export_manifest.json` at the root, or one this build cannot read.
    /// Not a failing disk and not a corrupt pack: a foreign file, which the
    /// `.mrpack` install path exists for.
    NotOurExport,
    /// F-121: the instance moved between the update plan and the confirm —
    /// the lock, the folder, or the target itself. Nothing was written; the
    /// user shows the plan again.
    UpdateContextChanged,
    /// F-123: installing without a profile backup was not confirmed a second
    /// time. The three-way question has no silent default.
    UpdateBackupNotConfirmed,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IpcError {
    pub code: ErrorCode,
    pub message: String,
    /// Omitted unless the provider was rate-limited; ts-rs would emit
    /// `bigint | null` and require the field, so the type is declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub retry_after_seconds: Option<u64>,
}

impl ErrorCode {
    /// The catalog key both locales carry for this code (`error.<camelCase>`).
    ///
    /// Serialized, not a second hand-written match: the generated binding the
    /// frontend consumes is produced from the same `serde` names, so the key
    /// cannot drift from the enum. Used where no frontend is running — the
    /// native startup dialog (F-139).
    pub fn i18n_key(self) -> String {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::String(name)) => format!("error.{name}"),
            _ => "error.internal".to_string(),
        }
    }
}

impl IpcError {
    pub fn new(code: ErrorCode, message: impl ToString) -> Self {
        Self {
            code,
            message: message.to_string(),
            retry_after_seconds: None,
        }
    }

    pub fn invalid_input(message: impl ToString) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub fn io(message: impl ToString) -> Self {
        Self::new(ErrorCode::Io, message)
    }

    pub fn internal(message: impl ToString) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    /// F-019: response bodies never cross IPC — only the typed `Display`.
    pub fn from_provider(e: &mm_core::providers::ProviderError) -> Self {
        let code = match e {
            mm_core::providers::ProviderError::Api { status } if *status == 404 => {
                ErrorCode::NotFound
            }
            // Missing or rejected credentials are actionable by the user, not a
            // generic provider refusal: the UI points at Settings.
            mm_core::providers::ProviderError::Api { status }
                if *status == 401 || *status == 403 =>
            {
                ErrorCode::ProviderUnauthorized
            }
            mm_core::providers::ProviderError::Api { .. } => ErrorCode::ProviderApi,
            mm_core::providers::ProviderError::Unsupported { .. } => ErrorCode::Unsupported,
            mm_core::providers::ProviderError::RateLimited { .. } => ErrorCode::RateLimited,
            mm_core::providers::ProviderError::Timeout => ErrorCode::Timeout,
            mm_core::providers::ProviderError::Transport
            | mm_core::providers::ProviderError::InvalidJson => ErrorCode::Io,
        };
        let mut err = Self::new(code, e);
        if let mm_core::providers::ProviderError::RateLimited { reset_seconds } = e {
            err.retry_after_seconds = Some(*reset_seconds);
        }
        err
    }

    /// A resolver failure carries the provider failure that caused it; the
    /// code is the provider's code, not a generic `Internal`.
    pub fn from_resolve(e: &mm_core::resolver::ResolveError) -> Self {
        Self::from_provider(e.provider_error())
    }

    pub fn from_lock(e: &mm_core::instances::LockError) -> Self {
        let code = match e {
            mm_core::instances::LockError::NewerSchema { .. } => ErrorCode::NewerSchema,
            mm_core::instances::LockError::Invalid => ErrorCode::InvalidInput,
            // Both the read and the atomic write are disk failures.
            mm_core::instances::LockError::Io(_) | mm_core::instances::LockError::Atomic(_) => {
                ErrorCode::Io
            }
        };
        Self::new(code, e)
    }

    /// A key-file failure (ADR 0004 §2-1-4). `SecretError`'s messages are fixed
    /// strings and paths and never contain the key, so echoing its `Display` is
    /// safe; it must stay that way — do not add a variant that quotes content.
    pub fn from_secret(e: &mm_core::providers::curseforge::secret::SecretError) -> Self {
        Self::new(ErrorCode::Internal, e)
    }
}

// --- search -------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchQueryDto {
    pub query: String,
    /// Which provider answers. Defaulted so an older caller cannot silently
    /// change meaning; the value is matched in `crate::providers`, never here.
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub game_version: Option<String>,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default = "default_project_type")]
    pub project_type: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_project_type() -> String {
    "mod".to_string()
}

fn default_provider() -> String {
    mm_core::providers::modrinth::PROVIDER_ID.to_string()
}

fn default_limit() -> u32 {
    20
}

impl SearchQueryDto {
    /// Core does not clamp; the bridge does (untrusted input).
    pub fn to_core(&self) -> mm_core::providers::SearchQuery {
        mm_core::providers::SearchQuery {
            query: self.query.trim().to_string(),
            game_version: self.game_version.clone(),
            loader: self.loader.clone(),
            project_type: if self.project_type.trim().is_empty() {
                default_project_type()
            } else {
                self.project_type.trim().to_string()
            },
            limit: self.limit.clamp(1, 50),
            offset: self.offset,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectDto {
    pub id: String,
    pub slug: Option<String>,
    pub title: String,
    pub description: String,
    pub author: String,
    pub project_type: String,
    pub categories: Vec<String>,
    #[ts(type = "number")]
    pub downloads: u64,
    /// F-165: the table's Followers column.
    #[ts(type = "number")]
    pub followers: u64,
    pub icon_url: Option<String>,
    /// Raw provider timestamp (never parsed): the newest version's date, else
    /// the project's own creation date.
    pub published: String,
    /// `required` | `optional` | `unsupported`, or empty when the provider does
    /// not state a side (CurseForge).
    pub client_side: String,
    pub server_side: String,
    /// The newest version's label, when the provider answered for it.
    pub latest_version: Option<String>,
    /// The project's own page, for a row's URL column.
    pub web_url: Option<String>,
    /// Provider identity from day one (`Provider::provider_id()`).
    pub provider_id: String,
}

impl ProjectDto {
    pub fn from_core(p: &mm_core::providers::Project, provider_id: &str) -> Self {
        Self {
            id: p.id.clone(),
            slug: p.slug.clone(),
            title: p.title.clone(),
            description: p.description.clone(),
            author: p.author.clone(),
            project_type: p.project_type.clone(),
            categories: p.categories.clone(),
            downloads: p.downloads,
            followers: p.followers,
            icon_url: p.icon_url.clone(),
            published: p.published.clone(),
            client_side: p.client_side.clone(),
            server_side: p.server_side.clone(),
            latest_version: p.latest_version.clone(),
            web_url: p.web_url.clone(),
            provider_id: provider_id.to_string(),
        }
    }
}

/// F-164: one row of the inspector's versions list.
///
/// Deliberately smaller than the install plan's entries: this answers "what
/// versions exist", not "what would a download do". Only the primary file's
/// name is carried (F-017 keeps a URL-less file, so the name is the honest
/// label even when there is nothing to download).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectVersionDto {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub date_published: String,
    pub version_type: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub filename: Option<String>,
}

impl ProjectVersionDto {
    pub fn from_core(v: &mm_core::providers::Version) -> Self {
        let primary = v
            .files
            .iter()
            .find(|f| f.primary)
            .or_else(|| v.files.first());
        Self {
            id: v.id.clone(),
            project_id: v.project_id.clone(),
            name: v.name.clone(),
            version_number: v.version_number.clone(),
            date_published: v.date_published.clone(),
            version_type: v.version_type.clone(),
            game_versions: v.game_versions.clone(),
            loaders: v.loaders.clone(),
            filename: primary.map(|f| f.filename.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchPageDto {
    pub hits: Vec<ProjectDto>,
    #[ts(type = "number")]
    pub total: u64,
}

// --- provider key (F-014, ADR 0004 §2-1-4) ------------------------------
//
// Settings shows the *source* of the live key and the path of the file, never
// the key itself (ADR 0004 §2-1-5). There is deliberately no key field on
// `CurseForgeKeyDto`: adding one would be the leak the ADR forbids.

/// Where the live CurseForge key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum KeySourceDto {
    File,
    Environment,
    None,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CurseForgeKeyDto {
    pub source: KeySourceDto,
    pub has_key: bool,
    /// `config_dir/curseforge.secret.json` — a path is not a secret, and the
    /// screen needs it to explain the roaming caveat (ADR 0004 §2-1-7).
    pub path: String,
}

// --- plan ---------------------------------------------------------------

/// `reason` / `message` are resolver-produced **data**, pinned by exact-string
/// core tests — not UI copy (§5.9). Some interpolate dynamic args (game
/// version, loader, counts), so a code alone cannot carry them; the UI shows
/// them verbatim inside a localized frame until Task 5 adds i18n.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlanEntryDto {
    pub source: String,
    /// `Provider::provider_id()` of the content this entry came from.
    pub provider: String,
    pub project_id: String,
    pub title: String,
    pub project_type: String,
    pub version_id: Option<String>,
    pub version_number: Option<String>,
    pub filename: Option<String>,
    /// `downloadable` | `blocked`.
    pub status: String,
    pub reason: String,
    pub dependency_type: String,
}

impl PlanEntryDto {
    pub fn from_core(e: &mm_core::resolver::PlanEntry) -> Self {
        Self {
            source: e.source.clone(),
            provider: e.provider.clone(),
            project_id: e.project_id.clone(),
            title: e.title.clone(),
            project_type: e.project_type.clone(),
            version_id: e.version_id.clone(),
            version_number: e.version_number.clone(),
            filename: e.filename.clone(),
            status: e.status.clone(),
            reason: e.reason.clone(),
            dependency_type: e.dependency_type.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlanSummaryDto {
    #[ts(type = "number")]
    pub selected_count: usize,
    #[ts(type = "number")]
    pub dependency_count: usize,
    #[ts(type = "number")]
    pub total_count: usize,
    #[ts(type = "number")]
    pub downloadable_count: usize,
    #[ts(type = "number")]
    pub blocked_count: usize,
    pub game_version: String,
    pub loader: String,
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WarningDto {
    /// `warning` | `info` — never blocks (F-066).
    pub severity: String,
    pub message: String,
}

impl WarningDto {
    pub fn from_core(w: &mm_core::resolver::Warning) -> Self {
        Self {
            severity: w.severity.clone(),
            message: w.message.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallPlanDto {
    pub entries: Vec<PlanEntryDto>,
    pub summary: PlanSummaryDto,
    pub warnings: Vec<WarningDto>,
}

// --- instance scan ------------------------------------------------------

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LocalFactDto {
    /// Filename only — absolute paths never cross IPC.
    pub filename: String,
    /// Path inside `mods/`, `/`-separated (`a.jar`, `disabled_mods/b.jar`).
    /// The filename alone cannot address a file the scan found in a
    /// subfolder, so toggle and remove take this value, never a bare name.
    pub rel_path: String,
    #[ts(type = "number")]
    pub size: u64,
    pub sha1: String,
    /// `mod` | `shaderpack` | `datapack` | `resourcepack` | `unknown`.
    pub resource_type: String,
    pub disabled: bool,
    /// The lock's project id for this file, when it has one.
    pub project_id: Option<String>,
    /// F-097/F-098: the one label the screen shows, derived in core from the
    /// axes below: `managed` | `externalUnknown` | `disabled` | `duplicate` |
    /// `incompatible`. The axes travel too — the label is not the truth.
    pub class: String,
    /// Ownership axis: `managed` | `external`.
    pub ownership: String,
    /// How ownership was proven: `sha1` | `provider_version` | `` (none).
    pub ownership_evidence: String,
    /// F-099: the identities this file shares with another file in the folder.
    pub duplicate_keys: Vec<DuplicateKeyDto>,
    /// Compatibility axis: `compatible` | `incompatible` | `unknown`.
    pub compatibility: String,
    /// Why: `noTarget` | `unproven` | `range` | `gameVersion` | `loader`;
    /// `null` when the file is compatible.
    pub compatibility_reason: Option<String>,
    /// The game version or loader the reason names.
    pub compatibility_target: Option<String>,
    /// F-100: the bytes no longer match the SHA-1 the lock records.
    pub hash_mismatch: bool,
}

impl LocalFactDto {
    pub fn from_fact(fact: &mm_core::instances::LocalContentFact) -> Self {
        let reason = fact.compatibility.reason();
        Self {
            filename: fact.filename.clone(),
            rel_path: fact.rel_path.clone(),
            size: fact.size,
            sha1: fact.sha1.clone(),
            resource_type: format!("{:?}", fact.resource_type).to_lowercase(),
            disabled: fact.disabled,
            project_id: if fact.project_id.is_empty() {
                None
            } else {
                Some(fact.project_id.clone())
            },
            class: fact.class().as_str().to_string(),
            ownership: fact.ownership.as_str().to_string(),
            ownership_evidence: fact.ownership.evidence().to_string(),
            duplicate_keys: fact.duplicate_keys.iter().map(Into::into).collect(),
            compatibility: fact.compatibility.as_str().to_string(),
            compatibility_reason: reason.map(|r| r.code().to_string()),
            compatibility_target: reason.and_then(|r| r.target()).map(str::to_string),
            hash_mismatch: fact.hash_mismatch,
        }
    }
}

/// One identity a file shares with another (F-099).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicateKeyDto {
    /// `sha1` | `project` — which identity the files share.
    pub kind: String,
    /// The full key, e.g. `sha1:…` or `provider_project:modrinth:…`.
    pub key: String,
}

impl From<&mm_core::instances::DuplicateKey> for DuplicateKeyDto {
    fn from(key: &mm_core::instances::DuplicateKey) -> Self {
        Self {
            kind: key.kind().to_string(),
            key: key.as_key(),
        }
    }
}

/// A folder-level finding the screen shows above the file list.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ContentWarningDto {
    /// `duplicateLocalIdentity` | `managedHashMismatch`.
    pub kind: String,
    /// `sha1` | `project`, for the duplicate kinds.
    pub key_kind: Option<String>,
    /// The file concerned; absent for a folder-wide duplicate key.
    pub rel_path: Option<String>,
    /// The shared identity, for the duplicate kinds.
    pub key: Option<String>,
}

impl ContentWarningDto {
    pub fn from_core(warning: &mm_core::instances::ContentWarning) -> Self {
        use mm_core::instances::ContentWarning;
        match warning {
            ContentWarning::DuplicateLocalIdentity(key) => Self {
                kind: warning.kind().to_string(),
                key_kind: Some(key.kind().to_string()),
                rel_path: None,
                key: Some(key.as_key()),
            },
            ContentWarning::ManagedHashMismatch { rel_path } => Self {
                kind: warning.kind().to_string(),
                key_kind: None,
                rel_path: Some(rel_path.clone()),
                key: None,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LockEntryDto {
    pub provider: String,
    pub project_id: String,
    pub slug: String,
    pub version_id: String,
    pub filename: String,
    pub sha1: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstanceSnapshotDto {
    pub loaders: Vec<String>,
    /// `launcher` | `jars` | `unknown` — F-092/F-093 guard provenance.
    pub loader_source: String,
    pub mods: Vec<LocalFactDto>,
    pub locked: Vec<LockEntryDto>,
    /// F-099/F-100: duplicates and changed files, folder-wide.
    pub content_warnings: Vec<ContentWarningDto>,
}

/// F-168: the watcher's typed signal. The event carries the root whose
/// content moved, never a file path — the UI only needs to know *which*
/// instance's scan went stale.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ContentStaleDto {
    pub instance_root: String,
}

// --- updates (F-120) -----------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckUpdatesRequest {
    pub instance_root: String,
    /// `Provider::provider_id()`; the bridge rejects anything else.
    pub provider: String,
    pub game_version: String,
    pub loader: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UpdateStatusDto {
    Available,
    UpToDate,
    Unidentified,
    Unsupported,
}

impl From<mm_core::updates::UpdateStatus> for UpdateStatusDto {
    fn from(status: mm_core::updates::UpdateStatus) -> Self {
        match status {
            mm_core::updates::UpdateStatus::Available => Self::Available,
            mm_core::updates::UpdateStatus::UpToDate => Self::UpToDate,
            mm_core::updates::UpdateStatus::Unidentified => Self::Unidentified,
            mm_core::updates::UpdateStatus::Unsupported => Self::Unsupported,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UpdateWarningDto {
    Unsupported,
    EmptyLock,
    LockUnreadable,
}

impl From<mm_core::updates::UpdateWarning> for UpdateWarningDto {
    fn from(warning: mm_core::updates::UpdateWarning) -> Self {
        match warning {
            mm_core::updates::UpdateWarning::Unsupported => Self::Unsupported,
            mm_core::updates::UpdateWarning::EmptyLock => Self::EmptyLock,
            mm_core::updates::UpdateWarning::LockUnreadable => Self::LockUnreadable,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateCandidateDto {
    pub filename: String,
    pub provider: String,
    pub project_id: String,
    pub current_version_id: String,
    pub current_sha1: String,
    pub current_sha512: String,
    pub latest_version_id: Option<String>,
    pub latest_version_number: Option<String>,
    pub latest_filename: Option<String>,
    pub latest_sha1: Option<String>,
    pub latest_sha512: Option<String>,
    pub status: UpdateStatusDto,
}

impl From<mm_core::updates::UpdateCandidate> for UpdateCandidateDto {
    fn from(c: mm_core::updates::UpdateCandidate) -> Self {
        Self {
            filename: c.filename,
            provider: c.provider,
            project_id: c.project_id,
            current_version_id: c.current_version_id,
            current_sha1: c.current_sha1,
            current_sha512: c.current_sha512,
            latest_version_id: c.latest_version_id,
            latest_version_number: c.latest_version_number,
            latest_filename: c.latest_filename,
            latest_sha1: c.latest_sha1,
            latest_sha512: c.latest_sha512,
            status: c.status.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdatePreviewDto {
    pub checked: usize,
    pub candidates: Vec<UpdateCandidateDto>,
    pub warnings: Vec<UpdateWarningDto>,
}

impl From<mm_core::updates::UpdatePreview> for UpdatePreviewDto {
    fn from(p: mm_core::updates::UpdatePreview) -> Self {
        Self {
            checked: p.checked,
            candidates: p.candidates.into_iter().map(Into::into).collect(),
            warnings: p.warnings.into_iter().map(Into::into).collect(),
        }
    }
}

// --- target planner (F-125..F-127, F-130..F-132) --------------------------

#[derive(Debug, Clone, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TargetPlanRequest {
    pub instance_root: String,
    pub provider: String,
    pub game_version: String,
    pub loader: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TargetStatusDto {
    UpToDate,
    Download,
    SyncOnly,
    Incompatible,
    OtherProvider,
    ProviderError,
}

impl From<mm_core::updates::target::TargetStatus> for TargetStatusDto {
    fn from(status: mm_core::updates::target::TargetStatus) -> Self {
        use mm_core::updates::target::TargetStatus as Core;
        match status {
            Core::UpToDate => Self::UpToDate,
            Core::Download => Self::Download,
            Core::SyncOnly => Self::SyncOnly,
            Core::Incompatible => Self::Incompatible,
            Core::OtherProvider => Self::OtherProvider,
            Core::Error => Self::ProviderError,
        }
    }
}

/// One row of the eight-column table. `reason` and the file names are payload
/// behind the columns, not columns of their own.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TargetRowDto {
    pub title: String,
    pub current_version: String,
    pub target_version: String,
    pub game_version: String,
    pub loader: String,
    pub download: bool,
    pub sync: bool,
    pub status: TargetStatusDto,
    pub reason_code: String,
    pub reason_detail: String,
    pub provider: String,
    pub project_id: String,
    pub slug: String,
    pub current_filename: String,
    pub target_filename: String,
    pub target_url: Option<String>,
    #[ts(type = "number | null")]
    pub target_size: Option<u64>,
    pub target_sha1: Option<String>,
    pub target_sha512: Option<String>,
    pub file_present: bool,
}

impl From<mm_core::updates::target::TargetRow> for TargetRowDto {
    fn from(row: mm_core::updates::target::TargetRow) -> Self {
        Self {
            title: row.title,
            current_version: row.current_version,
            target_version: row.target_version,
            game_version: row.game_version,
            loader: row.loader,
            download: row.download,
            sync: row.sync,
            status: row.status.into(),
            reason_code: row.reason.code.to_string(),
            reason_detail: row.reason.detail,
            provider: row.provider,
            project_id: row.project_id,
            slug: row.slug,
            current_filename: row.current_filename,
            target_filename: row.target_filename,
            target_url: row.target_url,
            target_size: row.target_size,
            target_sha1: row.target_sha1,
            target_sha512: row.target_sha512,
            file_present: row.file_present,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeptUnknownFileDto {
    pub rel_path: String,
    pub filename: String,
    pub sha1: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CleanupReportDto {
    pub kept_unknown: Vec<KeptUnknownFileDto>,
    pub recognized_incompatible: Vec<String>,
    pub present_files: usize,
    pub eligible_files: usize,
}

impl From<mm_core::updates::target::CleanupReport> for CleanupReportDto {
    fn from(report: mm_core::updates::target::CleanupReport) -> Self {
        Self {
            kept_unknown: report
                .kept_unknown
                .into_iter()
                .map(|kept| KeptUnknownFileDto {
                    rel_path: kept.rel_path,
                    filename: kept.filename,
                    sha1: kept.sha1,
                    reason: kept.reason.to_string(),
                })
                .collect(),
            recognized_incompatible: report.recognized_incompatible,
            present_files: report.present_files,
            eligible_files: report.eligible_files,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommonTargetSkipDto {
    pub project_id: String,
    pub title: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BestUpgradeDto {
    pub target: String,
    pub supported: usize,
    pub total: usize,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommonTargetDto {
    pub suggested: Option<String>,
    pub common_versions: Vec<String>,
    pub loader: String,
    pub checked: usize,
    pub skipped: Vec<CommonTargetSkipDto>,
    pub best_upgrade: Option<BestUpgradeDto>,
}

impl From<mm_core::updates::target::CommonTarget> for CommonTargetDto {
    fn from(common: mm_core::updates::target::CommonTarget) -> Self {
        Self {
            suggested: common.suggested,
            common_versions: common.common_versions,
            loader: common.loader,
            checked: common.checked,
            skipped: common
                .skipped
                .into_iter()
                .map(|skip| CommonTargetSkipDto {
                    project_id: skip.project_id,
                    title: skip.title,
                    reason: skip.reason.to_string(),
                })
                .collect(),
            best_upgrade: common.best_upgrade.map(|best| BestUpgradeDto {
                target: best.target,
                supported: best.supported,
                total: best.total,
                blockers: best.blockers,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TargetWarningDto {
    pub code: String,
    pub project_id: Option<String>,
}

impl From<mm_core::updates::target::TargetWarning> for TargetWarningDto {
    fn from(warning: mm_core::updates::target::TargetWarning) -> Self {
        let project_id = match &warning {
            mm_core::updates::target::TargetWarning::ProviderFailure { project_id } => {
                Some(project_id.clone())
            }
            _ => None,
        };
        Self {
            code: warning.code().to_string(),
            project_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PruneCandidateDto {
    pub filename: String,
    pub provider: String,
    pub project_id: String,
    pub version_id: String,
}

impl From<mm_core::updates::prune::PruneCandidate> for PruneCandidateDto {
    fn from(candidate: mm_core::updates::prune::PruneCandidate) -> Self {
        Self {
            filename: candidate.filename,
            provider: candidate.provider,
            project_id: candidate.project_id,
            version_id: candidate.version_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TargetPlanDto {
    pub game_version: String,
    pub loader: String,
    pub rows: Vec<TargetRowDto>,
    pub common: Option<CommonTargetDto>,
    pub cleanup: CleanupReportDto,
    pub prune_candidates: Vec<PruneCandidateDto>,
    pub warnings: Vec<TargetWarningDto>,
}

impl From<mm_core::updates::target::TargetPlan> for TargetPlanDto {
    fn from(plan: mm_core::updates::target::TargetPlan) -> Self {
        Self {
            game_version: plan.game_version,
            loader: plan.loader,
            rows: plan.rows.into_iter().map(Into::into).collect(),
            common: plan.common.map(Into::into),
            cleanup: plan.cleanup.into(),
            prune_candidates: plan.prune_candidates.into_iter().map(Into::into).collect(),
            warnings: plan.warnings.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrunePlanRequest {
    pub instance_root: String,
}

/// The mandatory dry run of F-130. `digest` is the confirming token; it covers
/// the lock's bytes and the candidate names, so a plan that would remove a
/// different set of records cannot reuse it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrunePlanDto {
    pub candidates: Vec<PruneCandidateDto>,
    pub dry_run: bool,
    pub present_files: usize,
    pub lock_entries: usize,
    pub digest: String,
}

impl From<mm_core::updates::prune::PrunePlan> for PrunePlanDto {
    fn from(plan: mm_core::updates::prune::PrunePlan) -> Self {
        let digest = plan.digest().to_string();
        Self {
            candidates: plan.candidates.into_iter().map(Into::into).collect(),
            dry_run: plan.dry_run,
            present_files: plan.present_files,
            lock_entries: plan.lock_entries,
            digest,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExecutePruneRequest {
    pub instance_root: String,
    /// The digest the dry run returned. A different one is a moved lock.
    pub expected_digest: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PruneReportDto {
    pub removed: Vec<String>,
    pub kept_present: Vec<String>,
    pub lock_entries_before: usize,
    pub lock_entries_after: usize,
}

impl From<mm_core::updates::prune::PruneReport> for PruneReportDto {
    fn from(report: mm_core::updates::prune::PruneReport) -> Self {
        Self {
            removed: report.removed,
            kept_present: report.kept_present,
            lock_entries_before: report.lock_entries_before,
            lock_entries_after: report.lock_entries_after,
        }
    }
}

// --- applying updates (F-121..F-124) -------------------------------------

/// One line of the confirmation copy. The UI renders `key` from its own
/// locale; `text` is the English fallback for a locale that lags behind.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SafetyContractItemDto {
    pub id: String,
    pub key: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdatePlanEntryDto {
    pub project_id: String,
    pub old_filename: String,
    pub filename: String,
    pub version_number: String,
    pub size: u64,
}

/// The plan the dialog shows: what will be replaced, and the token the confirm
/// must carry back (F-121).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateInstallPlanDto {
    pub token: String,
    pub entries: Vec<UpdatePlanEntryDto>,
    pub contract: Vec<SafetyContractItemDto>,
}

#[derive(Debug, Clone, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateInstallPlanRequest {
    pub instance_root: String,
    pub provider: String,
    pub game_version: String,
    pub loader: String,
    /// Project ids to install; empty means every candidate the preview marked
    /// `Available`.
    #[serde(default)]
    pub selected: Vec<String>,
}

/// The three-way answer to "create a full profile backup first?" (F-123).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BackupDecisionDto {
    Yes,
    No,
    Cancel,
}

#[derive(Debug, Clone, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApplyUpdateRequest {
    pub instance_root: String,
    pub provider: String,
    pub game_version: String,
    pub loader: String,
    #[serde(default)]
    pub selected: Vec<String>,
    /// The token `preview_update_install` returned. A mismatch is
    /// `UpdateContextChanged`: the instance moved, so the plan is stale.
    pub expected_token: String,
    /// Ignored by download-only, which touches nothing installed.
    pub backup: BackupDecisionDto,
    /// The second, explicit answer that makes `No` a decision rather than a
    /// default (F-123).
    #[serde(default)]
    pub confirm_no_backup: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstalledUpdateDto {
    pub project_id: String,
    pub filename: String,
    pub version_number: String,
    /// The replaced file inside the dated folder, when there was one.
    pub replaced: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SkippedUpdateDto {
    pub project_id: String,
    pub filename: String,
    /// `updates.skip.<reasonCode>` in both locales.
    pub reason_code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateInstallResultDto {
    pub operation_id: String,
    pub installed: Vec<InstalledUpdateDto>,
    pub skipped: Vec<SkippedUpdateDto>,
    pub replaced_backup_dir: String,
    pub profile_backup_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PreparedUpdateDto {
    pub project_id: String,
    pub filename: String,
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateDownloadResultDto {
    pub operation_id: String,
    pub directory: String,
    pub prepared: Vec<PreparedUpdateDto>,
}

// --- toggle -------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ToggleActionDto {
    Disable,
    Enable,
}

impl From<ToggleActionDto> for mm_core::instances::toggle::ToggleAction {
    fn from(a: ToggleActionDto) -> Self {
        match a {
            ToggleActionDto::Disable => Self::Disable,
            ToggleActionDto::Enable => Self::Enable,
        }
    }
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreviewToggleDto {
    /// The instance folder. The `mods` subfolder is derived here, not in the
    /// webview: the UI never assembles a filesystem path (`AGENTS.md` §5.2),
    /// and a separator guess would be wrong on one of the two platforms.
    pub instance_root: String,
    pub action: ToggleActionDto,
    /// Paths inside `<instance_root>/mods`, `/`-separated and relative
    /// (`a.jar`, `disabled_mods/a.jar`) — never absolute, never `..`.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PreviewEntryDto {
    pub filename: String,
    pub dest_filename: String,
    pub allowed: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TogglePlanDto {
    pub entries: Vec<PreviewEntryDto>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToggleResultDto {
    pub filename: String,
    pub dest_filename: String,
    /// `success` | `failed` | `skipped` | `rolledBack` | `rollbackFailed` | `rollbackSkipped`.
    pub state: String,
    pub reason: String,
}

// --- Safe content removal (F-111/F-112) ---------------------------------
//
// Removal is preview then execute, like toggle. The plan carries a stable
// reason code per entry so the UI translates it; `reason` stays the legacy
// English wording and is never the only record of a refusal. Execution
// rebuilds the plan in Rust: a body from the webview is not trusted.

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemovePreviewRequestDto {
    /// The instance folder; `mods` is derived in Rust.
    pub instance_root: String,
    /// Paths inside `mods`, `/`-separated, as returned by `scan_instance`.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoveExecuteRequestDto {
    pub instance_root: String,
    pub files: Vec<String>,
    /// The exact word for external/unknown entries; empty when not needed.
    #[serde(default)]
    pub confirmation: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoveEntryDto {
    pub filename: String,
    pub rel_path: String,
    pub allowed: bool,
    /// Allowed, but ownership is external/unknown: confirm with the word.
    pub requires_confirmation: bool,
    /// `allowed` | `root-missing` | `source-missing` | `symlink` | `outside`
    /// | `backup-folder` | `unsupported` | ...
    pub reason_code: String,
    /// English spec wording, for logs and for a code the UI does not know.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemovePlanDto {
    pub entries: Vec<RemoveEntryDto>,
    /// At least one allowed entry needs the typed word.
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoveResultDto {
    pub filename: String,
    pub rel_path: String,
    /// `moved-to-backup` | `failed`.
    pub state: String,
    /// `moved-to-backup` | guard code | `backup-exists` | `backup-unwritable`
    /// | `backup-outside` | `move-failed`.
    pub reason_code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoveRunDto {
    pub results: Vec<RemoveResultDto>,
    /// Empty when `remove_manifest.json` was written; otherwise why not.
    pub manifest_error: String,
}

// --- Instances (F-101..F-104) -------------------------------------------
//
// The UI sends intent, never a plan: `execute_create_instance` rebuilds the
// plan in Rust, so a folder taken between preview and confirm is refused by
// `plan_create` rather than trusted from the webview.

/// F-104 source as a real union, so the UI branches on a type instead of on
/// string equality. `mm_core::db::instances::InstanceSource` cannot derive TS:
/// core must not depend on ts-rs, so the DTO mirrors it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum InstanceSourceDto {
    Managed,
    Manual,
    Adopted,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstanceSummaryDto {
    pub id: String,
    pub name: String,
    /// Canonical instance root, as stored.
    pub path: String,
    pub minecraft_root: String,
    /// Options here are required-but-nullable, matching `ProjectDto.slug`:
    //  `#[ts(optional)]` is only for fields with `skip_serializing_if`.
    pub minecraft_version: Option<String>,
    pub loader: Option<String>,
    pub source: InstanceSourceDto,
    /// The registered folder is gone. The row is still listed: it is the
    /// user's registration, and only the user may delete it.
    pub path_missing: bool,
    /// `number | null`, not `number`: ts-rs emits `bigint` for i64, and a bare
    /// `type = "number"` would also drop the nullability — declaring a required
    /// number while Rust sends null for a never-scanned instance.
    #[ts(type = "number | null")]
    pub scanned_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateInstanceRequest {
    pub name: String,
    pub root: String,
    pub minecraft_version: Option<String>,
    pub loader: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LinkInstanceRequest {
    pub name: String,
    pub root: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateInstancePlanDto {
    /// The folder that will hold the instance.
    pub root: String,
    pub name: String,
    /// Absolute directories that will be created, root first.
    pub directories: Vec<String>,
    /// Relative folder names below the root, for display.
    pub relative_directories: Vec<String>,
}

/// One crashed instance create awaiting a decision (F-146).
///
/// `action` and `reason` are i18n keys, not display strings: core decides
/// what is true, the UI decides how to say it (AGENTS.md §5.9).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateRecoveryDto {
    pub operation_id: String,
    pub root: String,
    pub instance_id: String,
    /// `RecoveryAction::key()`.
    pub action: String,
    /// `RecoveryReason::key()`.
    pub reason: String,
}

use serde::Deserialize;

/// What the webview asks for, and what it learns (F-063).
///
/// Generated as a binding like every other request struct, so the webview
/// cannot drift from the Rust shape (AGENTS.md §5.3).
///
/// One provider per request: `projects` are slugs for Modrinth and numeric mod
/// ids for CurseForge, so a mixed list has no meaning.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallRequest {
    /// Absolute path of the instance the mods go into.
    pub instance_root: String,
    /// Slugs (Modrinth) or numeric mod ids (CurseForge) for `provider`.
    pub projects: Vec<String>,
    #[serde(default = "default_provider")]
    pub provider: String,
    pub game_version: String,
    pub loader: String,
}

/// One progress event on the install channel.
///
/// `phase` is an i18n key rather than a word, so core decides what is true and
/// the UI decides how to say it. `Option` without `skip_serializing_if` is
/// required-nullable in this house convention (ADR 0002).
///
/// F-020: `phase = "install.phase.batch"` marks the batch's own event, with
/// the batch totals in `done`/`total` and an empty `item_id` — not one file's
/// progress, and the per-item rows must not consume it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallEventDto {
    pub item_id: String,
    pub phase: String,
    pub done: Option<u64>,
    pub total: Option<u64>,
    pub path: Option<String>,
    pub message: Option<String>,
}

/// What an install returned once every item had settled.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallResultDto {
    /// The journal operation, so a restart can find this batch.
    pub operation_id: String,
    pub installed: Vec<String>,
    pub skipped: Vec<String>,
}

// --- download history (F-152, F-153) ------------------------------------

/// One row of the install log.
///
/// `status` is a stable identifier (`success` | `skipped` | `failed` |
/// `cancelled`), not a sentence: the UI translates `downloads.status.<value>`
/// and decides how a failure reads. `message` is diagnostic text, kept for the
/// row's own detail line and never shown as the whole state.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DownloadHistoryEntryDto {
    /// Row id. Exact as a JS number for any id this table will ever hold.
    #[ts(type = "number")]
    pub id: i64,
    pub provider: String,
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub version_id: Option<String>,
    pub version_number: Option<String>,
    pub game_version: String,
    pub loader: String,
    pub file_name: String,
    pub file_sha1: Option<String>,
    pub destination_path: Option<String>,
    pub status: String,
    pub message: String,
    /// Epoch seconds. The screen formats it in the user's locale; picking a
    /// zone is a display decision, not a storage one.
    #[ts(type = "number")]
    pub created_at: i64,
}

impl From<&mm_core::db::download_history::HistoryEntry> for DownloadHistoryEntryDto {
    fn from(e: &mm_core::db::download_history::HistoryEntry) -> Self {
        Self {
            id: e.id,
            provider: e.provider.clone(),
            project_id: e.project_id.clone(),
            slug: e.slug.clone(),
            title: e.title.clone(),
            version_id: e.version_id.clone(),
            version_number: e.version_number.clone(),
            game_version: e.game_version.clone(),
            loader: e.loader.clone(),
            file_name: e.file_name.clone(),
            file_sha1: e.file_sha1.clone(),
            destination_path: e.destination_path.clone(),
            status: e.status.as_str().to_string(),
            message: e.message.clone(),
            created_at: e.created_at,
        }
    }
}

/// What the history screen asks for. An empty `status` means "all"; the three
/// tabs differ only in this field.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadHistoryQueryDto {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub search: String,
    #[serde(default = "default_history_limit")]
    pub limit: u32,
}

fn default_history_limit() -> u32 {
    200
}

// --- Modpack (.mrpack) install ------------------------------------------

/// What the webview asks for: a pack file and the instance it goes into.
///
/// Plan and apply take the same shape on purpose. Apply rebuilds the plan from
/// these two paths and never accepts a plan body from the webview — the same
/// rule `execute_toggle` and `execute_create_instance` follow (AGENTS.md §5.4).
///
/// `pack_sha512` is the exception to "the paths are the whole request": it is
/// the digest [`ModpackPlanDto::pack_sha512`] handed back, and the apply
/// recomputes it and refuses a mismatch. Without it a `.mrpack` replaced
/// between the preview and the confirm would be applied unseen. Optional in
/// the type because the plan call cannot send what it is about to receive; the
/// apply treats an absent digest as a refusal, never as "whatever is there".
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModpackInstallRequest {
    /// Absolute path of the `.mrpack` the user chose. Validated in Rust.
    pub pack_path: String,
    /// Absolute path of the instance the pack is installed into.
    pub instance_root: String,
    /// SHA-512 of the pack file, lowercase hex, exactly as the plan returned it.
    #[serde(default)]
    #[ts(optional)]
    pub pack_sha512: Option<String>,
}

/// How one planned write produces its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PackWriteKindDto {
    /// Fetched by hash from the pack's URL or from the provider.
    Download,
    /// Copied out of the archive's override layer.
    Override,
}

/// One file the pack asks to be written.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PackWriteDto {
    /// Instance-relative and already validated by core; absolute paths never
    /// cross IPC for a file inside an instance.
    pub safe_path: String,
    pub kind: PackWriteKindDto,
    /// True when a file already sits at that destination and the apply will
    /// replace it. False means the apply creates it.
    pub overwrites: bool,
    /// Size the pack's index declared for a download, when it declared one.
    /// `number | null` for the same reason as `InstanceSummaryDto.scanned_at`:
    /// a bare `type = "number"` would also drop the nullability.
    #[ts(type = "number | null")]
    pub size: Option<u64>,
}

/// What the pack would do to an instance, before any of it happens.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModpackPlanDto {
    /// SHA-512 of the pack file these writes were read from, lowercase hex.
    ///
    /// The apply must send it back and is refused when the file no longer
    /// hashes to it: the plan is a promise about one archive, and this is the
    /// only thing that ties the two together once the webview holds it.
    pub pack_sha512: String,
    pub name: String,
    pub version_id: String,
    pub summary: Option<String>,
    pub writes: Vec<PackWriteDto>,
    /// Counts, so the screen can state the totals without walking the list.
    #[ts(type = "number")]
    pub download_count: usize,
    #[ts(type = "number")]
    pub override_count: usize,
    /// Destinations that already hold a file and will be replaced.
    #[ts(type = "number")]
    pub replace_count: usize,
    /// Remote entries with `env.client = "optional"`; this install has no
    /// per-file selection yet, so they are not applied (F-074).
    #[ts(type = "number")]
    pub skipped_optional: usize,
    /// Remote entries with `env.client = "unsupported"`: never applied on a
    /// client (F-074).
    #[ts(type = "number")]
    pub skipped_unsupported: usize,
    /// Files under `server-overrides/`: a client install never writes them
    /// (F-074).
    #[ts(type = "number")]
    pub skipped_server_overrides: usize,
}

/// What the export screen asks to be reviewed.
#[derive(Debug, Clone, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModpackExportRequest {
    /// Absolute path of the registered instance being exported.
    pub instance_root: String,
    pub settings: ModpackExportSettingsDto,
}

/// The user's choices for the pack the export writes.
#[derive(Debug, Clone, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModpackExportSettingsDto {
    pub name: String,
    /// The pack's own version id; the UI pre-fills a timestamp.
    pub version_id: String,
    pub summary: Option<String>,
    pub game_version: String,
    /// `fabric`, `quilt`, `forge` or `neoforge`; core maps it to the format key.
    pub loader: String,
    /// Instance-relative folders copied under `overrides/` (F-082).
    pub override_folders: Vec<String>,
}

/// The confirmed export: the same settings plus the destination and the token
/// that proves the instance did not move under the preview.
#[derive(Debug, Clone, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModpackExportWriteRequest {
    pub instance_root: String,
    pub settings: ModpackExportSettingsDto,
    /// `lockSha512` from the preview; a mismatch is `instanceChanged`.
    pub expected_lock_sha512: String,
    /// Absolute destination path the save dialog returned.
    pub output_path: String,
}

/// One resolved project the pack will reference.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModpackExportFileDto {
    pub project_id: String,
    pub version_id: String,
    pub title: String,
    pub project_type: String,
    /// Destination inside the pack (`mods/sodium.jar`).
    pub path: String,
    #[ts(type = "number")]
    pub size: u64,
}

/// One local file the pack will carry under `overrides/`.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModpackExportOverrideDto {
    pub archive_path: String,
    #[ts(type = "number")]
    pub size: u64,
}

/// One locked project left out, with the core's stable reason key.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModpackExportSkipDto {
    pub project_id: String,
    pub version_id: String,
    /// `unsupported_provider`, `no_file`, …; the UI translates
    /// `export.skip.<reason>`.
    pub reason: String,
}

/// One local path kept out of the pack, with core's stable reason key.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModpackExportExcludedDto {
    /// Instance-relative path, forward slashes.
    pub path: String,
    /// `transient`, `environment`, `secret`, `symlink`, `duplicate`,
    /// `unsafe_name`; the UI translates `export.excluded.<reason>`.
    pub reason: String,
}

/// Everything the pack will contain. Read-only: nothing was written to build
/// this answer (AGENTS.md §5.4).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModpackExportPlanDto {
    pub name: String,
    pub version_id: String,
    pub summary: Option<String>,
    pub game_version: String,
    pub loader: String,
    /// The token the write must send back; see `ModpackExportWriteRequest`.
    pub lock_sha512: String,
    pub files: Vec<ModpackExportFileDto>,
    pub overrides: Vec<ModpackExportOverrideDto>,
    pub skipped: Vec<ModpackExportSkipDto>,
    pub excluded: Vec<ModpackExportExcludedDto>,
    #[ts(type = "number")]
    pub override_count: usize,
    #[ts(type = "number")]
    pub total_override_bytes: u64,
}

/// What writing the pack produced.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModpackExportSummaryDto {
    pub output: String,
    #[ts(type = "number")]
    pub file_count: usize,
    #[ts(type = "number")]
    pub override_count: usize,
    #[ts(type = "number")]
    pub total_size: u64,
}

// --- Export archive import (F-158) ----------------------------------------

/// What the webview asks for when previewing an export archive.
#[derive(Debug, Clone, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportArchivePreviewRequest {
    /// Absolute path of the export archive the user chose. Validated in Rust.
    pub pack_path: String,
}

/// One path the manifest records as excluded, with core's stable reason key.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportArchiveExcludedDto {
    pub path: String,
    /// `transient`, `environment`, `secret`, `symlink`, `duplicate`,
    /// `unsafe_name`; the UI translates `export.excluded.<reason>`.
    pub reason: String,
}

/// The metadata-only preview of an export archive (F-158).
///
/// Nothing was extracted, written or planned to build this answer; the
/// manifest's audit record is the content. `pack_sha512` is the token the
/// import confirm sends back, the same contract `ModpackPlanDto` carries.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportArchivePreviewDto {
    /// SHA-512 of the archive file, lowercase hex.
    pub pack_sha512: String,
    /// The manifest schema version the archive carries.
    #[ts(type = "number")]
    pub manifest_schema_version: u64,
    pub name: String,
    pub version_id: String,
    pub summary: Option<String>,
    pub game_version: String,
    pub loader: String,
    /// The digest the exporter recorded for the source instance's lock.
    pub lock_sha512: String,
    #[ts(type = "number")]
    pub file_count: usize,
    #[ts(type = "number")]
    pub override_count: usize,
    #[ts(type = "number")]
    pub skipped_count: usize,
    pub excluded: Vec<ExportArchiveExcludedDto>,
    /// The paths the manifest records as included index files: what the
    /// import will download. The write plan the user sees before confirming.
    pub included_files: Vec<String>,
    /// The paths the manifest records as included overrides: what the import
    /// will copy out of the archive.
    pub included_overrides: Vec<String>,
}

/// What the import confirm asks for: the archive, the new instance's folder
/// and name, and the digest token the preview returned.
///
/// No plan body travels back: the create plan is rebuilt in Rust from the
/// folder and name, and the pack apply re-reads and re-plans the archive —
/// the same rule every execute command follows.
#[derive(Debug, Clone, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportArchiveImportRequest {
    pub pack_path: String,
    /// Absolute path of the folder the new instance is created in. It must
    /// not exist or be empty; `plan_create` refuses anything else.
    pub instance_root: String,
    pub instance_name: String,
    /// `packSha512` from the preview; a mismatch is `packChanged`.
    pub pack_sha512: String,
}

/// One progress event on the modpack channel.
///
/// `phase` is one of core's `pack::exec::phase` keys (`pack.phase.*`), never a
/// display word: core says what is true, the UI says how to say it. Numbers
/// are annotated as `number` because that is what JSON carries; ts-rs would
/// otherwise emit `bigint` for `u64`.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PackEventDto {
    pub phase: String,
    pub safe_path: Option<String>,
    #[ts(type = "number")]
    pub done: u64,
    #[ts(type = "number | null")]
    pub total: Option<u64>,
}

/// What applying a pack did, once every op settled.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModpackApplyResultDto {
    /// Files this run wrote.
    #[ts(type = "number")]
    pub written: usize,
    /// Destinations that already held the right bytes, so nothing was written.
    #[ts(type = "number")]
    pub skipped: usize,
    /// Bytes written, as counted by core's apply report.
    #[ts(type = "number")]
    pub bytes: u64,
}

// --- Pack recovery (F-076/F-146) -----------------------------------------
//
// A crashed or failed pack apply left the instance half applied. Core assesses
// the record and the disk and offers the user a decision; the bridge carries
// that decision as data. The operation id is the only token the UI may send
// back: everything else — the root, the pack, the staging tree — comes from the
// record, so the webview can never name a folder for core to act on.

/// One interrupted pack apply awaiting a decision.
///
/// `action` and `reason` are i18n keys, not display strings: core decides what
/// is true, the UI decides how to say it (AGENTS.md §5.9). No user-readable
/// sentence may live in a DTO field.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PackRecoveryDto {
    /// The journal operation the UI sends back to resume or dismiss it.
    pub operation_id: String,
    /// The record's instance root, for display only — never sent back.
    pub instance_root: String,
    pub pack_name: String,
    pub pack_version_id: String,
    /// The digest of the archive the plan was built from, for display.
    pub pack_sha512: String,
    /// How many destinations the plan names.
    #[ts(type = "number")]
    pub planned_count: usize,
    /// How many of them are in place — the evidence from the disk, not the
    /// record's own counters.
    #[ts(type = "number")]
    pub done_count: usize,
    /// `PackRecoveryAction::key()` — e.g. `recovery.pack.action.resume`.
    pub action: String,
    /// `PackRecoveryReason::key()` — e.g. `recovery.pack.partiallyApplied`.
    pub reason: String,
}

/// What a recovery run did, once it ended.
///
/// `failed` and `state` are why a recovery reports itself separately from
/// [`ModpackApplyResultDto`]: a failed recovery leaves the record `running` and
/// the case open (F-076), and that is something the screen must be able to say.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PackRecoveryResultDto {
    pub operation_id: String,
    /// Files this run wrote.
    #[ts(type = "number")]
    pub written: usize,
    /// Files that were already in place and were left alone.
    #[ts(type = "number")]
    pub skipped: usize,
    /// Files that could not be written.
    #[ts(type = "number")]
    pub failed: usize,
    /// `planned | running | completed | failed | cancelled` — the record's
    /// state when the run ended. `running` means the case is still open: the
    /// instance is half applied and the banner keeps offering the decision.
    pub state: String,
}

// --- The unified operations view (F-145) ----------------------------------
//
// Read-only, across every kind the journal records. Each row is the record's
// own fields plus a reason key when it is unfinished; the assessment is what
// the disk holds plus the action key of the surface that already owns the
// kind. The webview sends an operation id and nothing else.

/// One row of the operations table.
///
/// `kind` and `state` are the lowercase words the record itself serializes to
/// (`download_batch`, `mrpack_install`, `running`, …): the UI branches and
/// groups on them, and translates `operations.kind.*` / `operations.state.*`
/// (AGENTS.md §5.9 — no display sentence crosses IPC).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OperationSummaryDto {
    /// The journal operation the UI sends back to the assessor.
    pub operation_id: String,
    /// `download_batch | update_download | update_install | mrpack_install |
    /// instance_create`.
    pub kind: String,
    /// `planned | running | completed | failed | cancelled`.
    pub state: String,
    /// Epoch seconds, as the record stores them.
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    /// The instance the operation belongs to; empty for a batch that has none.
    pub profile_id: String,
    /// Where the operation acts: an instance root, or the download target.
    pub target_path: String,
    #[ts(type = "number")]
    pub planned_count: usize,
    #[ts(type = "number")]
    pub completed_count: usize,
    #[ts(type = "number")]
    pub failed_count: usize,
    #[ts(type = "number")]
    pub cancelled_count: usize,
    #[ts(type = "number")]
    pub skipped_count: usize,
    /// The pack's display name for a pack apply, empty for every other kind.
    pub pack_name: String,
    /// An i18n reason key when the record is unfinished, empty when terminal.
    pub reason: String,
}

/// What one record's disk holds, and who may act on it.
///
/// The counters are evidence from the disk (`final_present`, `part_present`),
/// not the record's own tallies: the record is one write late by definition.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OperationAssessmentDto {
    pub operation_id: String,
    pub kind: String,
    pub state: String,
    pub target_path: String,
    /// Whether the record's staging directory exists.
    pub staging_present: bool,
    /// Whether the record's backup directory exists.
    pub backup_present: bool,
    /// How many items the record carries.
    #[ts(type = "number")]
    pub item_count: usize,
    /// How many items' final destinations exist on disk.
    #[ts(type = "number")]
    pub final_present: usize,
    /// How many items' `.part` files exist on disk.
    #[ts(type = "number")]
    pub part_present: usize,
    /// The i18n action key of the recovery surface that already owns this
    /// kind; empty when none exists — the view then stays strictly read-only.
    pub action: String,
    /// An i18n reason key, empty when the record is terminal.
    pub reason: String,
}

/// One record the scan could not read: a corrupt file, a foreign record, a
/// symlink the scan refused to follow.
///
/// The old centre reported these instead of dropping them silently. Nothing is
/// settled or repaired here — the file is named so the user can look at it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OperationIssueDto {
    /// Basename only; full paths stay out of the UI.
    pub filename: String,
    /// Why it could not be read, as the journal store phrased it: diagnostic
    /// text, not a translated sentence.
    pub message: String,
}

/// One scan: the table's rows and the files that could not become rows.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OperationsListingDto {
    /// Every record the journal holds, newest first — terminal ones included.
    pub rows: Vec<OperationSummaryDto>,
    /// Records the scan could not read, named and left untouched.
    pub issues: Vec<OperationIssueDto>,
}

// --- Legacy import (F-181) -----------------------------------------------
//
// A scan is a report, not a plan and not a promise. Every path here is for
// display only: nothing in this DTO is ever accepted back as an instruction,
// and the scan never writes into the old home. `source`, the warning `code`
// and the JSON `detail` are stable codes; the UI owns the sentences.

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LegacyScanDto {
    pub roots: LegacyRootsDto,
    pub database: Option<LegacyDatabaseDto>,
    pub profiles: Vec<LegacyProfileDto>,
    pub warnings: Vec<LegacyWarningDto>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LegacyRootsDto {
    /// `explicit | environment | legacy_dot_dir | platform | none`.
    pub source: String,
    pub data: String,
    pub config: String,
    pub cache: String,
    pub logs: String,
    pub db_file: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LegacyDatabaseDto {
    /// The original file, never the snapshot: the snapshot is gone by the time
    /// this DTO exists.
    pub path: String,
    /// Bytes of the original file, for the diagnostic line.
    #[ts(type = "number")]
    pub bytes: u64,
    pub user_version: u32,
    pub supported: bool,
    pub has_profiles_table: bool,
    /// Discovered with `PRAGMA table_info`, so a v1 file lists 23 columns and
    /// a v2 file 34. Displayed as evidence, not interpreted here.
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LegacyProfileDto {
    pub id: String,
    pub name: String,
    pub game_version: String,
    pub loader: String,
    /// `profile_kind` on a v2 database; empty on v1.
    pub kind: String,
    /// `ownership_policy` on a v2 database; empty on v1.
    pub ownership: String,
    pub root: String,
    pub root_exists: bool,
    pub profile_json: LegacyJsonFactDto,
    pub installed_mods: LegacyInstalledModsFactDto,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LegacyJsonFactDto {
    pub present: bool,
    pub schema: String,
    pub ok: bool,
    /// `missing | invalid_json | unsupported_schema | not_an_object | .` -
    /// a code, never a sentence.
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LegacyInstalledModsFactDto {
    pub present: bool,
    pub schema: String,
    pub ok: bool,
    /// Records an import would consider: managed status and both identities
    /// present.
    #[ts(type = "number")]
    pub managed: usize,
    /// Records that cannot be matched to a provider and would be left alone.
    #[ts(type = "number")]
    pub skipped: usize,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LegacyWarningDto {
    /// `root_not_found | database_missing | database_unreadable | .`.
    pub code: String,
    /// The profile the warning belongs to, when it belongs to one row.
    pub profile: Option<String>,
    /// Paths and parser reasons for the diagnostic line. Never translated and
    /// never parsed by the UI.
    pub detail: String,
}

impl From<&mm_core::legacy::LegacyScan> for LegacyScanDto {
    fn from(scan: &mm_core::legacy::LegacyScan) -> Self {
        Self {
            roots: LegacyRootsDto::from(&scan.roots),
            database: scan.database.as_ref().map(LegacyDatabaseDto::from),
            profiles: scan.profiles.iter().map(LegacyProfileDto::from).collect(),
            warnings: scan.warnings.iter().map(LegacyWarningDto::from).collect(),
        }
    }
}

impl From<&mm_core::legacy::LegacyRoots> for LegacyRootsDto {
    fn from(roots: &mm_core::legacy::LegacyRoots) -> Self {
        Self {
            source: roots.source.code().to_string(),
            data: display(&roots.data),
            config: display(&roots.config),
            cache: display(&roots.cache),
            logs: display(&roots.logs),
            db_file: display(&roots.db_file),
        }
    }
}

impl From<&mm_core::legacy::LegacyDbScan> for LegacyDatabaseDto {
    fn from(db: &mm_core::legacy::LegacyDbScan) -> Self {
        Self {
            path: display(&db.path),
            bytes: db.bytes,
            user_version: db.user_version,
            supported: db.supported,
            has_profiles_table: db.has_profiles_table,
            columns: db.columns.clone(),
        }
    }
}

impl From<&mm_core::legacy::LegacyProfileScan> for LegacyProfileDto {
    fn from(profile: &mm_core::legacy::LegacyProfileScan) -> Self {
        Self {
            id: profile.id.clone(),
            name: profile.name.clone(),
            game_version: profile.game_version.clone(),
            loader: profile.loader.clone(),
            kind: profile.kind.clone(),
            ownership: profile.ownership.clone(),
            root: display(&profile.root),
            root_exists: profile.root_exists,
            profile_json: LegacyJsonFactDto::from(&profile.profile_json),
            installed_mods: LegacyInstalledModsFactDto::from(&profile.installed_mods),
        }
    }
}

impl From<&mm_core::legacy::JsonFact> for LegacyJsonFactDto {
    fn from(fact: &mm_core::legacy::JsonFact) -> Self {
        Self {
            present: fact.present,
            schema: fact.schema.clone(),
            ok: fact.ok,
            detail: fact.detail.clone(),
        }
    }
}

impl From<&mm_core::legacy::InstalledModsFact> for LegacyInstalledModsFactDto {
    fn from(fact: &mm_core::legacy::InstalledModsFact) -> Self {
        Self {
            present: fact.present,
            schema: fact.schema.clone(),
            ok: fact.ok,
            managed: fact.managed,
            skipped: fact.skipped,
            detail: fact.detail.clone(),
        }
    }
}

impl From<&mm_core::legacy::LegacyWarning> for LegacyWarningDto {
    fn from(warning: &mm_core::legacy::LegacyWarning) -> Self {
        Self {
            code: warning.code.to_string(),
            profile: warning.profile.clone(),
            detail: warning.detail.clone(),
        }
    }
}

// --- Legacy import: plan and execute (F-181) ------------------------------
//
// `action` (`register | skip | block`), every `code` and `blockedReason` are
// stable codes: the UI owns the sentences and translates them. `detail` is a
// path or an id for the diagnostic line - never parsed, never translated.
// `entries` in the report mirror the plan's order, so a row the user saw in
// the preview is the same row in the result.

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LegacyImportPlanDto {
    pub entries: Vec<LegacyImportPlanEntryDto>,
    #[ts(type = "number")]
    pub ready: usize,
    #[ts(type = "number")]
    pub skipped: usize,
    #[ts(type = "number")]
    pub blocked: usize,
    /// `root_not_found | database_missing | database_unsupported |
    /// profiles_table_missing`. Set only when no entry can be planned at all.
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LegacyImportPlanEntryDto {
    pub legacy_id: String,
    pub name: String,
    /// The detected instance root this entry would register. Empty when the
    /// profile has no usable root.
    pub root: String,
    pub game_version: Option<String>,
    pub loader: Option<String>,
    /// `managed | manual | adopted`.
    pub source: String,
    /// `register | skip | block`.
    pub action: String,
    /// `ready | already_registered | root_missing | duplicate_in_selection |
    /// profile_missing`.
    pub code: String,
    /// The instance that already owns `root`, when one does.
    pub existing_instance: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LegacyImportReportDto {
    /// The backup of the new database taken before the first row, when rows
    /// were written. `None` means nothing was written, so nothing needed one.
    pub backup: Option<String>,
    #[ts(type = "number")]
    pub imported: usize,
    #[ts(type = "number")]
    pub skipped: usize,
    #[ts(type = "number")]
    pub blocked: usize,
    pub blocked_reason: Option<String>,
    pub entries: Vec<LegacyImportOutcomeDto>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LegacyImportOutcomeDto {
    pub legacy_id: String,
    pub name: String,
    /// True only for a row this run inserted.
    pub applied: bool,
    /// `imported` for an applied row; otherwise the plan's own code.
    pub code: String,
    pub instance_id: Option<String>,
    /// A path or an id, for the diagnostic line only.
    pub detail: String,
}

impl From<&mm_core::legacy::ImportPlan> for LegacyImportPlanDto {
    fn from(plan: &mm_core::legacy::ImportPlan) -> Self {
        Self {
            entries: plan.entries.iter().map(Into::into).collect(),
            ready: plan.ready,
            skipped: plan.skipped,
            blocked: plan.blocked,
            blocked_reason: plan.blocked_reason.map(str::to_string),
        }
    }
}

impl From<&mm_core::legacy::ImportEntry> for LegacyImportPlanEntryDto {
    fn from(entry: &mm_core::legacy::ImportEntry) -> Self {
        Self {
            legacy_id: entry.legacy_id.clone(),
            name: entry.name.clone(),
            root: display(&entry.root),
            game_version: entry.game_version.clone(),
            loader: entry.loader.clone(),
            source: entry.source.as_str().to_string(),
            action: action_code(entry.action).to_string(),
            code: entry.code.to_string(),
            existing_instance: entry.existing_instance.clone(),
        }
    }
}

impl From<&mm_core::legacy::ImportReport> for LegacyImportReportDto {
    fn from(report: &mm_core::legacy::ImportReport) -> Self {
        Self {
            backup: report.backup.as_deref().map(display),
            imported: report.imported,
            skipped: report.skipped,
            blocked: report.blocked,
            blocked_reason: report.blocked_reason.map(str::to_string),
            entries: report.outcomes.iter().map(Into::into).collect(),
        }
    }
}

impl From<&mm_core::legacy::ImportOutcome> for LegacyImportOutcomeDto {
    fn from(outcome: &mm_core::legacy::ImportOutcome) -> Self {
        Self {
            legacy_id: outcome.legacy_id.clone(),
            name: outcome.name.clone(),
            applied: outcome.applied,
            code: outcome.code.to_string(),
            instance_id: outcome.instance_id.clone(),
            detail: outcome.detail.clone(),
        }
    }
}

fn action_code(action: mm_core::legacy::ImportAction) -> &'static str {
    match action {
        mm_core::legacy::ImportAction::Register => "register",
        mm_core::legacy::ImportAction::Skip => "skip",
        mm_core::legacy::ImportAction::Block => "block",
    }
}

/// Display only. A path that is not valid UTF-8 is shown lossily; nothing on
/// this screen is ever sent back as a path.
fn display(path: &std::path::Path) -> String {
    path.display().to_string()
}

// --- backup & restore (F-113, F-114, F-115, F-118) ------------------------
//
// The dated folder on disk is the whole truth: no table, no index, no cached
// row, so every DTO below is read from a folder or written by a run that just
// finished. Paths cross IPC for display only; the two a command takes back —
// the instance root and the chosen backup folder — are gated in Rust
// (`registered_root`, then containment inside the backup root).
//
// `manifestState` is a word, not a sentence (`present`/`missing`/`legacy`/
// `invalid`): the screen decides how to say why a folder is not restorable, and
// a legacy backup is listed on purpose — hiding a folder the user can see in
// their file manager is the worse lie.

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupListDto {
    pub backup_root: String,
    pub backups: Vec<BackupCandidateDto>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupCandidateDto {
    pub stamp: String,
    pub created_at: String,
    pub include_saves: bool,
    /// `present`, `missing`, `legacy` or `invalid`.
    pub manifest_state: String,
    /// The manifest names this instance. False for another instance's backup
    /// and for a folder with no readable manifest at all.
    pub instance_matches: bool,
    #[ts(type = "number")]
    pub entry_count: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
}

/// Ask for a backup. `includeSaves` absent means `false`: the flag is stated,
/// never inferred from `saves/` existing.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupRequest {
    pub instance_root: String,
    #[serde(default)]
    #[ts(optional)]
    pub include_saves: Option<bool>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupResultDto {
    pub backup_dir: String,
    pub created_at: String,
    pub manifest_path: String,
    pub manifest_sha256: String,
    pub include_saves: bool,
    #[ts(type = "number")]
    pub copied_files: u64,
    #[ts(type = "number")]
    pub copied_bytes: u64,
    pub skipped: Vec<BackupSkipDto>,
}

/// One sector the backup states it does not hold, with a code the UI
/// translates (`source-missing`, `symlink`, `saves-not-included`).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupSkipDto {
    pub source: String,
    pub path: String,
    pub reason_code: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestorePreviewRequest {
    pub instance_root: String,
    /// A folder inside this instance's backup root. Core canonicalizes both
    /// sides and refuses anything else.
    pub backup_dir: String,
    #[serde(default)]
    #[ts(optional)]
    pub include_saves: Option<bool>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestorePlanDto {
    pub backup_dir: String,
    pub created_at: String,
    pub safety_dir: String,
    pub include_saves: bool,
    /// The freshness token: the SHA-256 of the manifest these entries were
    /// read from. The execute command must send it back.
    pub manifest_sha256: String,
    pub allowed: Vec<RestoreEntryDto>,
    pub blocked: Vec<RestoreEntryDto>,
}

/// Everything the user must send back with the typed word.
/// `expectedManifestSha256` is the plan's token; `confirmation` is `RESTORE`,
/// checked again in Rust.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreExecuteRequest {
    pub instance_root: String,
    pub backup_dir: String,
    #[serde(default)]
    pub include_saves: bool,
    pub expected_manifest_sha256: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreEntryDto {
    pub source: String,
    pub backup_path: String,
    pub target_path: String,
    pub safety_path: String,
    #[ts(type = "number")]
    pub file_count: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
    pub allowed: bool,
    /// `allowed`, `profile-not-restored`, `source-missing`, `entry-is-link`,
    /// `saves-blocked-by-default`, `entry-outside-group`, `safety-exists`,
    /// `target-under-link`. A blocked row is shown, never silently dropped.
    pub reason_code: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreResultEntryDto {
    pub source: String,
    pub backup_path: String,
    pub target_path: String,
    pub safety_path: String,
    /// `restored`, `failed`, `rolled-back`, `rollback-failed`, `cancelled`,
    /// `dependency-failed`. `rollback-failed` means the group failed AND the
    /// safety copy could not return: the target is missing or half written, and
    /// the screen must send the user to `safety_path` instead of saying the
    /// group was put back.
    pub state: String,
    pub reason_code: String,
    #[ts(type = "number")]
    pub file_count: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreRunDto {
    /// `complete`, `partial` or `cancelled`.
    pub outcome: String,
    pub safety_dir: String,
    pub manifest_path: String,
    pub entries: Vec<RestoreResultEntryDto>,
    pub blocked: Vec<RestoreEntryDto>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorldsPreviewDto {
    pub saves_dir: String,
    pub saves_exists: bool,
    pub worlds: Vec<WorldDto>,
    /// `worlds.no-saves`, `worlds.empty` or `worlds.read-only`.
    pub message_code: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorldDto {
    pub name: String,
    /// `detected`, `missing-level-dat` or `symlink`.
    pub status: String,
    #[ts(type = "number")]
    pub datapack_count: u64,
    pub has_level_dat: bool,
    /// `worlds.level-dat-found`, `worlds.level-dat-missing` or
    /// `worlds.symlink-skipped`.
    pub message_code: String,
}

/// A safety folder with no completed manifest: the restore that died mid-run.
/// Nothing is repaired automatically; the banner says where the old target is
/// and lets the user decide.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PendingRestoreDto {
    pub stamp: String,
    pub safety_dir: String,
    pub has_current: bool,
    pub groups: Vec<String>,
    pub message_code: String,
}

/// One progress event on the backup channel.
///
/// `phase` is an i18n key (`backup.phase.*`), and `terminal` is explicit so the
/// throttle never drops the event that ends the run.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupEventDto {
    pub phase: String,
    #[ts(type = "number")]
    pub copied_files: u64,
    #[ts(type = "number")]
    pub total_files: u64,
    #[ts(type = "number")]
    pub copied_bytes: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
    pub terminal: bool,
}

/// One progress event on the restore channel: the same shape, restored counts.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreEventDto {
    pub phase: String,
    #[ts(type = "number")]
    pub restored_files: u64,
    #[ts(type = "number")]
    pub total_files: u64,
    #[ts(type = "number")]
    pub restored_bytes: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
    pub terminal: bool,
}

impl From<&mm_core::instances::backup::BackupCandidate> for BackupCandidateDto {
    fn from(candidate: &mm_core::instances::backup::BackupCandidate) -> Self {
        Self {
            stamp: candidate.stamp.clone(),
            created_at: candidate.created_at.clone(),
            include_saves: candidate.include_saves,
            manifest_state: candidate.manifest_state.as_str().to_string(),
            instance_matches: candidate.instance_matches,
            entry_count: candidate.entry_count,
            total_bytes: candidate.total_bytes,
        }
    }
}

impl From<&mm_core::instances::backup::BackupRun> for BackupResultDto {
    fn from(run: &mm_core::instances::backup::BackupRun) -> Self {
        Self {
            backup_dir: display(&run.backup_dir),
            created_at: run.created_at.clone(),
            manifest_path: display(&run.manifest_path),
            manifest_sha256: run.manifest_sha256.clone(),
            include_saves: run.include_saves,
            copied_files: run.copied_files,
            copied_bytes: run.copied_bytes,
            skipped: run.skipped.iter().map(Into::into).collect(),
        }
    }
}

impl From<&mm_core::instances::backup::BackupSkip> for BackupSkipDto {
    fn from(skip: &mm_core::instances::backup::BackupSkip) -> Self {
        Self {
            source: skip.source.as_str().to_string(),
            path: skip.path.clone(),
            reason_code: skip.reason_code.clone(),
        }
    }
}

impl From<&mm_core::instances::backup::restore::RestoreEntry> for RestoreEntryDto {
    fn from(entry: &mm_core::instances::backup::restore::RestoreEntry) -> Self {
        Self {
            source: entry.source.as_str().to_string(),
            backup_path: display(&entry.backup_path),
            target_path: display(&entry.target_path),
            safety_path: display(&entry.safety_path),
            file_count: entry.file_count,
            total_bytes: entry.total_bytes,
            allowed: entry.allowed,
            reason_code: entry.reason_code.to_string(),
        }
    }
}

impl From<&mm_core::instances::backup::restore::RestoreResultEntry> for RestoreResultEntryDto {
    fn from(entry: &mm_core::instances::backup::restore::RestoreResultEntry) -> Self {
        Self {
            source: entry.source.as_str().to_string(),
            backup_path: display(&entry.backup_path),
            target_path: display(&entry.target_path),
            safety_path: display(&entry.safety_path),
            state: entry.state.as_str().to_string(),
            reason_code: entry.reason_code.clone(),
            file_count: entry.file_count,
            total_bytes: entry.total_bytes,
        }
    }
}

impl From<&mm_core::instances::backup::restore::RestorePlan> for RestorePlanDto {
    fn from(plan: &mm_core::instances::backup::restore::RestorePlan) -> Self {
        Self {
            backup_dir: display(&plan.backup_dir),
            created_at: plan.created_at.clone(),
            safety_dir: display(&plan.safety_dir),
            include_saves: plan.include_saves,
            manifest_sha256: plan.manifest_sha256.clone(),
            allowed: plan
                .entries
                .iter()
                .filter(|e| e.allowed)
                .map(Into::into)
                .collect(),
            blocked: plan
                .entries
                .iter()
                .filter(|e| !e.allowed)
                .map(Into::into)
                .collect(),
        }
    }
}

impl From<&mm_core::instances::backup::restore::RestoreRun> for RestoreRunDto {
    fn from(run: &mm_core::instances::backup::restore::RestoreRun) -> Self {
        Self {
            outcome: run.outcome.as_str().to_string(),
            safety_dir: display(&run.safety_dir),
            manifest_path: display(&run.manifest_path),
            entries: run.entries.iter().map(Into::into).collect(),
            blocked: run.blocked.iter().map(Into::into).collect(),
        }
    }
}

impl From<&mm_core::instances::backup::worlds::WorldEntry> for WorldDto {
    fn from(world: &mm_core::instances::backup::worlds::WorldEntry) -> Self {
        Self {
            name: world.name.clone(),
            status: world.status.as_str().to_string(),
            datapack_count: world.datapack_count,
            has_level_dat: world.has_level_dat,
            message_code: world.message_code.to_string(),
        }
    }
}

impl From<&mm_core::instances::backup::worlds::WorldsPreview> for WorldsPreviewDto {
    fn from(preview: &mm_core::instances::backup::worlds::WorldsPreview) -> Self {
        Self {
            saves_dir: display(&preview.saves_dir),
            saves_exists: preview.saves_exists,
            worlds: preview.worlds.iter().map(Into::into).collect(),
            message_code: preview.message_code.to_string(),
        }
    }
}

impl From<&mm_core::instances::backup::restore::PendingRestore> for PendingRestoreDto {
    fn from(pending: &mm_core::instances::backup::restore::PendingRestore) -> Self {
        Self {
            stamp: pending.stamp.clone(),
            safety_dir: display(&pending.safety_dir),
            has_current: pending.has_current,
            groups: pending.groups.clone(),
            // One key for the whole surface: the banner says the restore
            // stopped mid-run, and the group list says where.
            message_code: "restore.pending-incomplete".to_string(),
        }
    }
}
