//! Applying an [`InstallPlan`] to an instance directory.
//!
//! `archive` turns a `.mrpack` into a plan; this module is the only place in
//! the modpack path that writes. The order is the plan's own format order:
//! every override first (the bytes are already in the extracted archive), then
//! every download.
//!
//! Two rules shape the whole file. First, nothing is written outside
//! `instance_root` and nothing is written to a path the plan did not name:
//! every destination goes through [`safe_member_name`] again, even though
//! `archive` validated it, because a plan is data that can cross a process
//! boundary and a caller could hand us one it built itself. Second, a file is
//! committed only after its bytes match the SHA-512 in the index; a URL is a
//! hint, the hash is the gate (`AGENTS.md` §5.5).
//!
//! shortcut: no dependency resolution. This applies exactly the hashes the
//! index lists. A pack that forgets its libraries installs without them.
//! Ceiling: an install that looks complete but does not launch. Upgrade path:
//! hand `Pack.dependencies` to `resolver` and append its downloads to the plan
//! before applying.
//!
//! shortcut: layer precedence is a plain last-one-wins copy. `client-
//! overrides`, then `overrides`, then `server-overrides`, each replacing the
//! previous file; the plan already collapsed duplicate paths, so there is one
//! op per path and the source is the last layer that actually holds it.
//! Ceiling: a server install and a client install get the same files. Upgrade
//! path: filter layers by the requested `Side` once `archive` learns to take a
//! side, then keep this loop unchanged.
//!
//! shortcut: no `profile.json`. The instance metadata an installer usually
//! writes (loader, game version, icon) is not this module's format to invent.
//! Ceiling: the files are in place but the instance manager cannot point at
//! the pack as their source. Upgrade path: write it in `instances` once that
//! module owns the profile shape.
//!
//! shortcut: no crash recovery for a half-applied plan. Each file is atomic,
//! so a crash leaves whole files and possibly a `.mm-staging` leftover, never
//! a torn one; but nothing records which ops finished. Ceiling: a retry
//! re-copies overrides and re-hashes downloads (an already-valid destination
//! is skipped by hash), which is correct if wasteful. Upgrade path: journal
//! each op through `journal`, exactly as `downloader::execute_batch` does.

use std::collections::{BTreeSet, HashMap};
use std::io;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use super::archive::{InstallOp, InstallPlan};
use super::mrpack::Pack;
use crate::downloader::client::DownloadError;
use crate::downloader::{DownloadPhase, DownloadProgress, DownloadRequest, Downloader};
use crate::providers::{HashAlgo, Provider};
use crate::safety::atomic::{part_path, write_atomic, AtomicError};
use crate::safety::path::{safe_member_name, NameError};

/// Where a download is staged before it earns its destination.
///
/// Inside the instance root on purpose: the free-space preflight and the final
/// copy stay on one volume, and a failed run leaves nothing on `C:`. This is
/// the one path written outside the plan, and every file in it is removed as
/// soon as it is committed or rejected.
const STAGING_DIR: &str = ".mm-staging";

/// Progress phases as i18n keys, never as display words (`AGENTS.md` §5.9).
pub mod phase {
    /// An override is being copied into place.
    pub const COPYING: &str = "pack.phase.copying";
    /// Bytes are arriving.
    pub const DOWNLOADING: &str = "pack.phase.downloading";
    /// The hash is being checked.
    pub const VERIFYING: &str = "pack.phase.verifying";
    /// The verified file is being renamed into place.
    pub const COMMITTING: &str = "pack.phase.committing";
    /// One file is at its destination.
    pub const WRITTEN: &str = "pack.phase.written";
    /// The destination already held the right bytes; nothing was written.
    pub const SKIPPED: &str = "pack.phase.skipped";
    /// Terminal: the whole plan is applied.
    pub const DONE: &str = "pack.phase.done";
    /// Terminal: the plan stopped at the first failure.
    pub const FAILED: &str = "pack.phase.failed";
}

/// One progress event, small enough to serialize per call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackEvent {
    /// One of the [`phase`] constants: an i18n key the UI translates.
    pub phase: &'static str,
    /// Instance-relative path this event is about, if any.
    pub safe_path: Option<String>,
    /// Bytes done, for download events.
    pub done: u64,
    /// Bytes expected, when the server or the index said.
    pub total: Option<u64>,
}

impl PackEvent {
    fn operation(phase: &'static str) -> Self {
        Self {
            phase,
            safe_path: None,
            done: 0,
            total: None,
        }
    }

    fn file(phase: &'static str, safe_path: &str) -> Self {
        Self {
            phase,
            safe_path: Some(safe_path.to_string()),
            done: 0,
            total: None,
        }
    }

    fn progress(phase: &'static str, safe_path: &str, done: u64, total: Option<u64>) -> Self {
        Self {
            phase,
            safe_path: Some(safe_path.to_string()),
            done,
            total,
        }
    }
}

/// Where a written file came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppliedSource {
    /// Copied out of the extracted pack, from this override layer.
    Override { layer: String },
    /// Downloaded and hash-verified.
    Download,
}

/// One file this run actually wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedFile {
    pub safe_path: String,
    pub bytes: u64,
    pub source: AppliedSource,
}

/// What applying a plan did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyReport {
    pub written: Vec<AppliedFile>,
    /// Destinations that already held the right bytes, so nothing was written.
    pub skipped: Vec<String>,
    pub bytes: u64,
}

/// One write a dry run would perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedWrite {
    pub safe_path: String,
    pub kind: PlannedWriteKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedWriteKind {
    Override,
    Download { sha512: String, size: Option<u64> },
}

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    // `NameError` is Debug-only, same convention as `mrpack.rs`.
    #[error("plan destination {path:?} is unsafe: {reason:?}")]
    UnsafePath { path: String, reason: NameError },
    #[error("plan destination {path:?} is a directory, not a file")]
    NotAFile { path: String },
    #[error("destination {} escapes the instance root", .0.display())]
    Escape(PathBuf),
    #[error("override {path:?} is not in the extracted pack tree")]
    MissingOverride { path: String },
    #[error("no trusted source for {path:?}: neither the provider nor the pack offers a URL for {sha512}")]
    NoDownloadSource { path: String, sha512: String },
    #[error("hash mismatch for {path:?}: expected {expected}, got {actual}")]
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error(transparent)]
    Atomic(#[from] AtomicError),
    #[error(transparent)]
    Download(#[from] DownloadError),
    #[error("io {op} on {}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("internal: {0}")]
    Internal(&'static str),
}

/// Lists what applying `plan` would write, without touching the disk.
///
/// The paths get the same validation the real run gives them, so an unsafe
/// plan is refused here too rather than only halfway through an install.
pub fn dry_run(plan: &InstallPlan) -> Result<Vec<PlannedWrite>, ExecError> {
    let mut out = Vec::with_capacity(plan.ops.len());
    for op in &plan.ops {
        match op {
            InstallOp::CopyOverride { safe_path } => out.push(PlannedWrite {
                safe_path: checked_relative(safe_path)?,
                kind: PlannedWriteKind::Override,
            }),
            InstallOp::Download {
                safe_path,
                sha512,
                size,
            } => out.push(PlannedWrite {
                safe_path: checked_relative(safe_path)?,
                kind: PlannedWriteKind::Download {
                    sha512: sha512.clone(),
                    size: *size,
                },
            }),
        }
    }
    Ok(out)
}

/// Applies one plan. Borrows its inputs so the caller keeps ownership of the
/// shared clients and of the pack it parsed.
pub struct PlanExecutor<'a> {
    provider: &'a dyn Provider,
    downloader: &'a Downloader,
    pack: &'a Pack,
    plan: &'a InstallPlan,
    staging_root: &'a Path,
    instance_root: &'a Path,
    allow_insecure_loopback: bool,
}

impl<'a> PlanExecutor<'a> {
    /// `staging_root` is the directory `extract_safely` filled from the pack;
    /// override members live at `<staging_root>/<layer>/<safe_path>`.
    pub fn new(
        provider: &'a dyn Provider,
        downloader: &'a Downloader,
        pack: &'a Pack,
        plan: &'a InstallPlan,
        staging_root: &'a Path,
        instance_root: &'a Path,
    ) -> Self {
        Self {
            provider,
            downloader,
            pack,
            plan,
            staging_root,
            instance_root,
            allow_insecure_loopback: false,
        }
    }

    /// Tests serve wiremock over loopback `http`; production leaves this off so
    /// the downloader stays https-only.
    pub fn allow_insecure_loopback(mut self, allow: bool) -> Self {
        self.allow_insecure_loopback = allow;
        self
    }

    /// The list of writes an [`apply`](Self::apply) would perform.
    pub fn dry_run(&self) -> Result<Vec<PlannedWrite>, ExecError> {
        dry_run(self.plan)
    }

    /// Applies the plan, reporting through `on_event`.
    ///
    /// The last event is always terminal: [`phase::DONE`] on success,
    /// [`phase::FAILED`] on the first error. A failure stops the run; files
    /// already committed stay committed, because re-copying them is safe and
    /// deleting them would not be.
    ///
    /// `Send` on the callback is a consequence of the downloader wanting a
    /// `Send` progress closure; the IPC layer's channel is `Send` anyway.
    pub async fn apply(
        &self,
        on_event: &mut (dyn FnMut(&PackEvent) + Send),
    ) -> Result<ApplyReport, ExecError> {
        let mut report = ApplyReport::default();
        match self.apply_ops(&mut report, on_event).await {
            Ok(()) => {
                on_event(&PackEvent::operation(phase::DONE));
                Ok(report)
            }
            Err(e) => {
                on_event(&PackEvent::operation(phase::FAILED));
                Err(e)
            }
        }
    }

    async fn apply_ops(
        &self,
        report: &mut ApplyReport,
        on_event: &mut (dyn FnMut(&PackEvent) + Send),
    ) -> Result<(), ExecError> {
        for op in &self.plan.ops {
            if let InstallOp::CopyOverride { safe_path } = op {
                self.apply_override(safe_path, report, on_event)?;
            }
        }

        let downloads: Vec<&InstallOp> = self
            .plan
            .ops
            .iter()
            .filter(|op| matches!(op, InstallOp::Download { .. }))
            .collect();
        if downloads.is_empty() {
            return Ok(());
        }

        // One provider round-trip for the whole plan: `/version_files` takes a
        // hash array, and a pack routinely asks for dozens of files.
        let resolved = self.resolved_urls().await;
        for op in downloads {
            if let InstallOp::Download {
                safe_path,
                sha512,
                size,
            } = op
            {
                let urls = self.candidate_urls(safe_path, sha512, &resolved);
                self.apply_download(safe_path, sha512, *size, &urls, report, on_event)
                    .await?;
            }
        }
        Ok(())
    }

    fn apply_override(
        &self,
        safe_path: &str,
        report: &mut ApplyReport,
        on_event: &mut (dyn FnMut(&PackEvent) + Send),
    ) -> Result<(), ExecError> {
        let rel = checked_relative(safe_path)?;
        let dest = destination(self.instance_root, &rel)?;
        reject_link_ancestors(self.instance_root, &dest)?;
        let (layer, src) = self.override_source(&rel)?;

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err("create_dir", parent, e))?;
        }
        on_event(&PackEvent::file(phase::COPYING, &rel));
        let mut f = std::fs::File::open(&src).map_err(|e| io_err("open", &src, e))?;
        let bytes = write_atomic(&dest, &mut f)?;
        report.written.push(AppliedFile {
            safe_path: rel.clone(),
            bytes,
            source: AppliedSource::Override { layer },
        });
        report.bytes += bytes;
        on_event(&PackEvent::file(phase::WRITTEN, &rel));
        Ok(())
    }

    /// The last layer in `pack.override_layers` that actually holds the path
    /// wins: that is the format's order, and the plan collapsed duplicates
    /// without remembering which layer supplied the content.
    fn override_source(&self, rel: &str) -> Result<(String, PathBuf), ExecError> {
        let mut found: Option<(String, PathBuf)> = None;
        for layer in &self.pack.override_layers {
            let candidate = self.staging_root.join(layer).join(rel);
            if candidate.is_file() {
                found = Some((layer.clone(), candidate));
            }
        }
        found.ok_or_else(|| ExecError::MissingOverride {
            path: rel.to_string(),
        })
    }

    /// Maps a lowercased SHA-512 to the URL of the provider file with that
    /// hash. Provider failure is not fatal here: the pack's own URLs are still
    /// hash-gated, so a provider outage degrades instead of blocking.
    async fn resolved_urls(&self) -> HashMap<String, String> {
        let hashes: BTreeSet<String> = self
            .plan
            .ops
            .iter()
            .filter_map(|op| match op {
                InstallOp::Download { sha512, .. } => Some(sha512.to_lowercase()),
                InstallOp::CopyOverride { .. } => None,
            })
            .collect();
        if hashes.is_empty() {
            return HashMap::new();
        }
        let list: Vec<String> = hashes.into_iter().collect();
        match self
            .provider
            .get_versions_by_hashes(&list, HashAlgo::Sha512, None)
            .await
        {
            Ok(versions) => versions
                .into_iter()
                .filter_map(|(hash, version)| {
                    let matching = |f: &&crate::providers::ProjectFile| {
                        f.sha512
                            .as_deref()
                            .is_some_and(|h| h.eq_ignore_ascii_case(&hash))
                            && !f.url.is_empty()
                    };
                    // Primary first, then any hash match: a version can carry
                    // several files and only one is the one the pack named.
                    let pick = version
                        .files
                        .iter()
                        .find(|f| f.primary && matching(f))
                        .or_else(|| version.files.iter().find(matching))?;
                    Some((hash.to_lowercase(), pick.url.clone()))
                })
                .collect(),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "could not resolve pack files by hash; falling back to the pack's URLs"
                );
                HashMap::new()
            }
        }
    }

    /// Provider URL first, then the pack's own list in order, deduplicated.
    fn candidate_urls(
        &self,
        safe_path: &str,
        sha512: &str,
        resolved: &HashMap<String, String>,
    ) -> Vec<String> {
        let mut urls = Vec::new();
        if let Some(url) = resolved.get(&sha512.to_lowercase()) {
            urls.push(url.clone());
        }
        // Only a pack file whose hash matches this op may contribute URLs; a
        // mismatched entry is a different file with the same destination.
        if let Some(file) = self
            .pack
            .files
            .iter()
            .find(|f| f.safe_path == safe_path && f.sha512.eq_ignore_ascii_case(sha512))
        {
            for url in &file.downloads {
                if !url.is_empty() && !urls.contains(url) {
                    urls.push(url.clone());
                }
            }
        }
        urls
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_download(
        &self,
        safe_path: &str,
        sha512: &str,
        size: Option<u64>,
        urls: &[String],
        report: &mut ApplyReport,
        on_event: &mut (dyn FnMut(&PackEvent) + Send),
    ) -> Result<(), ExecError> {
        let rel = checked_relative(safe_path)?;
        let dest = destination(self.instance_root, &rel)?;
        reject_link_ancestors(self.instance_root, &dest)?;
        let want = sha512.to_lowercase();

        // F-032's spirit at the plan level: the right bytes already there are
        // not downloaded twice.
        if dest.is_file() && sha512_of(&dest).await? == want {
            report.skipped.push(rel.clone());
            on_event(&PackEvent::file(phase::SKIPPED, &rel));
            return Ok(());
        }
        if urls.is_empty() {
            return Err(ExecError::NoDownloadSource {
                path: rel,
                sha512: want,
            });
        }

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err("create_dir", parent, e))?;
        }
        let staging = self.instance_root.join(STAGING_DIR);
        std::fs::create_dir_all(&staging).map_err(|e| io_err("create_dir", &staging, e))?;
        let staged = staging_target(&staging, &want, &dest);

        let mut last_error: Option<ExecError> = None;
        for url in urls {
            // Fresh staging per attempt: a `.part` written by one URL must
            // never be resumed from a different URL.
            let _ = std::fs::remove_file(&staged);
            let _ = std::fs::remove_file(part_path(&staged));

            let request = DownloadRequest {
                url: url.clone(),
                dest: staged.clone(),
                expected_size: size,
                sha1_hex: None,
                sha512_hex: Some(want.clone()),
                allow_insecure_loopback: self.allow_insecure_loopback,
            };
            // The staged name keeps the destination's extension so the
            // downloader's archive sanity still runs for `.jar` payloads.
            let mut progress = |p: DownloadProgress| {
                let key = match p.phase {
                    DownloadPhase::Downloading => phase::DOWNLOADING,
                    DownloadPhase::Verifying => phase::VERIFYING,
                    DownloadPhase::Committing => phase::COMMITTING,
                };
                on_event(&PackEvent::progress(key, &rel, p.done, p.total));
                ControlFlow::Continue(())
            };
            let is_paused = || false;
            let outcome = self
                .downloader
                .download(&request, &mut progress, &is_paused)
                .await;
            drop(progress);

            match outcome {
                Ok(outcome) => {
                    // Defence in depth: the downloader verified the stream,
                    // this re-reads what actually landed before it is allowed
                    // to become the instance's file.
                    let actual = sha512_of(&outcome.path).await?;
                    if actual != want {
                        let _ = std::fs::remove_file(&outcome.path);
                        last_error = Some(ExecError::HashMismatch {
                            path: rel.clone(),
                            expected: want.clone(),
                            actual,
                        });
                        continue;
                    }
                    let mut f = std::fs::File::open(&outcome.path)
                        .map_err(|e| io_err("open", &outcome.path, e))?;
                    let bytes = write_atomic(&dest, &mut f)?;
                    let _ = std::fs::remove_file(&outcome.path);
                    let _ = std::fs::remove_dir(&staging);
                    report.written.push(AppliedFile {
                        safe_path: rel.clone(),
                        bytes,
                        source: AppliedSource::Download,
                    });
                    report.bytes += bytes;
                    on_event(&PackEvent::file(phase::WRITTEN, &rel));
                    return Ok(());
                }
                Err(e) => last_error = Some(ExecError::Download(e)),
            }
        }

        // Hash rejections delete their `.part`; an empty staging dir is noise,
        // so it goes too. A transport failure may keep a resumable `.part`,
        // which is why `remove_dir` (empty only) is the right call.
        let _ = std::fs::remove_dir(&staging);
        Err(last_error.unwrap_or(ExecError::NoDownloadSource {
            path: rel,
            sha512: want,
        }))
    }
}

/// Re-runs the plan's path gate. `archive` already did this when it built the
/// plan; doing it again is cheap and closes the hole where a plan arrives from
/// somewhere other than `archive::plan_install`.
fn checked_relative(safe_path: &str) -> Result<String, ExecError> {
    let normalized = safe_member_name(safe_path).map_err(|reason| ExecError::UnsafePath {
        path: safe_path.to_string(),
        reason,
    })?;
    if normalized.ends_with('/') {
        return Err(ExecError::NotAFile {
            path: safe_path.to_string(),
        });
    }
    Ok(normalized)
}

fn destination(instance_root: &Path, rel: &str) -> Result<PathBuf, ExecError> {
    let dest = instance_root.join(rel);
    if !dest.starts_with(instance_root) {
        return Err(ExecError::Escape(dest));
    }
    Ok(dest)
}

/// A pre-existing symlink or NTFS junction between the root and the target
/// would redirect the write outside the instance. `extract.rs` has the same
/// check; it is private there, and a one-screen helper is cheaper than making
/// the safety module's internals public.
fn reject_link_ancestors(instance_root: &Path, dest: &Path) -> Result<(), ExecError> {
    let mut ancestor = dest.parent();
    while let Some(a) = ancestor {
        if a == instance_root {
            break;
        }
        if is_link_or_reparse(a) {
            return Err(ExecError::Escape(a.to_path_buf()));
        }
        ancestor = a.parent();
    }
    Ok(())
}

fn is_link_or_reparse(path: &Path) -> bool {
    match path.symlink_metadata() {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return true;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
                (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
            }
            #[cfg(not(windows))]
            {
                false
            }
        }
        Err(_) => false,
    }
}

/// Sibling-free staging name that still ends in the destination's extension.
fn staging_target(staging_root: &Path, sha512: &str, dest: &Path) -> PathBuf {
    let prefix = sha512.get(..16).unwrap_or(sha512);
    let name: String = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download")
        .chars()
        .take(120)
        .collect();
    staging_root.join(format!("{prefix}-{name}"))
}

async fn sha512_of(path: &Path) -> Result<String, ExecError> {
    let owned = path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || sha512_of_file(&owned))
        .await
        .map_err(|_| ExecError::Internal("hash task"))?;
    result.map_err(|e| io_err("read", path, e))
}

fn sha512_of_file(path: &Path) -> io::Result<String> {
    use sha2::Digest as _;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha512::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = io::Read::read(&mut file, &mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn io_err(op: &'static str, path: &Path, source: io::Error) -> ExecError {
    ExecError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}
