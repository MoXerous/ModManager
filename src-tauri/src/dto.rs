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

/// رمز الخطأ المرسل للواجهة؛ تترجمه الواجهة إلى نص حسب لغة المستخدم.
/// `message` للتشخيص فقط (سجلّ) ولا يُعرض أبدًا.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ErrorCode {
    Internal,
    InvalidInput,
    NotFound,
    /// 404 من المزوّد.
    ProviderApi,
    RateLimited,
    /// `retry_after_seconds` يحمل الأوقات؛ الواجهة لا تحلّل نصًا.
    Timeout,
    Io,
    /// مخطط القفل أحدث من المدعوم (§5.10) — لا تُعرض قائمة فارغة.
    NewerSchema,
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
            mm_core::providers::ProviderError::Api { .. } => ErrorCode::ProviderApi,
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

    pub fn from_lock(e: &mm_core::instances::LockError) -> Self {
        let code = match e {
            mm_core::instances::LockError::NewerSchema { .. } => ErrorCode::NewerSchema,
            mm_core::instances::LockError::Invalid => ErrorCode::InvalidInput,
            mm_core::instances::LockError::Io(_) => ErrorCode::Io,
        };
        Self::new(code, e)
    }
}

// --- search -------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchQueryDto {
    pub query: String,
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
    #[ts(type = "number")]
    pub downloads: u64,
    pub icon_url: Option<String>,
    /// Provider identity from day one (only Modrinth today).
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
            downloads: p.downloads,
            icon_url: p.icon_url.clone(),
            provider_id: provider_id.to_string(),
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
    #[ts(type = "number")]
    pub size: u64,
    pub sha1: String,
    /// `mod` | `shaderpack` | `datapack` | `resourcepack` | `unknown`.
    pub resource_type: String,
    pub disabled: bool,
    pub project_id: Option<String>,
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
    /// Filenames inside `<instance_root>/mods`, not absolute paths.
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
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallRequest {
    /// Absolute path of the instance the mods go into.
    pub instance_root: String,
    pub slugs: Vec<String>,
    pub game_version: String,
    pub loader: String,
}

/// One progress event on the install channel.
///
/// `phase` is an i18n key rather than a word, so core decides what is true and
/// the UI decides how to say it. `Option` without `skip_serializing_if` is
/// required-nullable in this house convention (ADR 0002).
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
