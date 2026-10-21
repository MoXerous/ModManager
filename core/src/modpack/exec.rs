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
//! previous file; the plan already collapsed duplicate paths and names the
//! winning layer, so this module copies exactly that layer's bytes.
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
//! F-076: the apply is journalled. Every destination the plan names gets an
//! item in the record, carrying the `.part` this apply owns and the path it
//! lands at, and the record exists before the first byte moves — so a crash
//! leaves a record that says what was planned, what landed, and which
//! scratch file is ours. A retry re-copies overrides and re-hashes downloads
//! (an already-valid destination is skipped by hash), which is correct if
//! wasteful; the record is what lets a later run *resume* rather than guess.
//! The staging directory is removed at the start of every apply, so a killed
//! run cannot feed a later one, and at the end of every apply, success or
//! failure.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io;
use std::io::Read;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use super::archive::{InstallOp, InstallPlan};
use super::mrpack::{reserved_destination, Pack, STAGING_DIR};
use crate::downloader::client::DownloadError;
use crate::downloader::{DownloadPhase, DownloadProgress, DownloadRequest, Downloader};
use crate::journal::{
    pack_item, ItemState, Journal, JournalError, JournalOp, OperationItem, OperationKind,
    OperationRecord, OperationState, PackApplyMeta,
};
use crate::providers::{HashAlgo, Provider};
use crate::safety::atomic::{commit_part, part_path, write_part, AtomicError};
use crate::safety::path::{safe_member_name, NameError};

/// How many times removing the staging directory is retried. A scanner or an
/// antivirus can hold a handle on a fresh file for a moment on Windows; the
/// same reason `safety::atomic` retries its renames.
const STAGING_REMOVE_ATTEMPTS: u32 = 3;
const STAGING_REMOVE_WAIT_MS: u64 = 50;

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

/// The identity of the pack being applied (F-076).
///
/// The journal has to know *which* pack a record is about: a record that
/// names no archive cannot be resumed against one, and a later run has to be
/// able to tell that the pack in front of the user is the pack the record
/// belongs to. The archive's own digest is not in the index — it is computed
/// from the bytes the plan was built from — so it travels beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackIdentity {
    /// The pack's display name, as the index spells it.
    pub name: String,
    /// The pack's version id, as the index spells it.
    pub version_id: String,
    /// SHA-512 of the archive, lowercase hex.
    pub archive_sha512: String,
    /// Absolute path of the archive.
    pub archive_path: String,
}

impl PackIdentity {
    /// The journal's shape of this pack.
    pub fn to_meta(&self) -> PackApplyMeta {
        PackApplyMeta {
            pack_name: self.name.clone(),
            pack_version_id: self.version_id.clone(),
            archive_sha512: self.archive_sha512.clone(),
            archive_path: self.archive_path.clone(),
        }
    }
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
    /// True when a file already sits at the destination and the apply will
    /// replace it; false when the apply creates it. A download whose bytes
    /// already match is skipped instead of rewritten, so this is existence,
    /// not "the bytes will change". There is no backup: the plan is the
    /// user's chance to see the replacement before it happens.
    pub overwrites: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedWriteKind {
    Override,
    Download { sha512: String, size: Option<u64> },
}

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    // `NameError` is Debug-only, same convention as `mrpack.rs`.
    #[error("plan path {path:?} is unsafe: {reason:?}")]
    UnsafePath { path: String, reason: NameError },
    #[error("plan destination {path:?} is a directory, not a file")]
    NotAFile { path: String },
    #[error("plan destination {path:?} is reserved by the application")]
    ReservedPath { path: String },
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
    /// F-076: the record could not be written. The apply stops: a file
    /// nobody has a record of is a file no later run can reason about.
    #[error(transparent)]
    Journal(#[from] JournalError),
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

/// Lists what applying `plan` to `instance_root` would write, without touching
/// the disk.
///
/// The paths get the same validation the real run gives them, so an unsafe
/// plan is refused here too rather than only halfway through an install.
/// `overwrites` is answered from the filesystem: the user sees which
/// destinations replace an existing file before anything is written.
pub fn dry_run(plan: &InstallPlan, instance_root: &Path) -> Result<Vec<PlannedWrite>, ExecError> {
    let mut out = Vec::with_capacity(plan.ops.len());
    for op in &plan.ops {
        match op {
            InstallOp::CopyOverride { safe_path, .. } => {
                let rel = checked_relative(safe_path)?;
                let overwrites = destination(instance_root, &rel)?.is_file();
                out.push(PlannedWrite {
                    safe_path: rel,
                    kind: PlannedWriteKind::Override,
                    overwrites,
                });
            }
            InstallOp::Download {
                safe_path,
                sha512,
                size,
            } => {
                let rel = checked_relative(safe_path)?;
                let overwrites = destination(instance_root, &rel)?.is_file();
                out.push(PlannedWrite {
                    safe_path: rel,
                    kind: PlannedWriteKind::Download {
                        sha512: sha512.clone(),
                        size: *size,
                    },
                    overwrites,
                });
            }
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
    /// F-076: which pack this apply is about. The journal needs it, and the
    /// journal is what makes a crash resumable.
    identity: PackIdentity,
    allow_insecure_loopback: bool,
}

impl<'a> PlanExecutor<'a> {
    /// `staging_root` is the directory `extract_safely` filled from the pack;
    /// override members live at `<staging_root>/<layer>/<safe_path>`.
    ///
    /// `identity` names the archive the plan was built from (F-076). It is
    /// not optional: a pack apply without a record of *which* pack it was
    /// leaves a half-applied instance that nothing can resume, which is the
    /// failure this whole path exists to remove.
    pub fn new(
        provider: &'a dyn Provider,
        downloader: &'a Downloader,
        pack: &'a Pack,
        plan: &'a InstallPlan,
        staging_root: &'a Path,
        instance_root: &'a Path,
        identity: PackIdentity,
    ) -> Self {
        Self {
            provider,
            downloader,
            pack,
            plan,
            staging_root,
            instance_root,
            identity,
            allow_insecure_loopback: false,
        }
    }

    /// Tests serve wiremock over loopback `http`; production leaves this off so
    /// the downloader stays https-only.
    pub fn allow_insecure_loopback(mut self, allow: bool) -> Self {
        self.allow_insecure_loopback = allow;
        self
    }

    /// The list of writes an [`apply`](Self::apply) would perform, including
    /// which destinations replace a file that is already there.
    pub fn dry_run(&self) -> Result<Vec<PlannedWrite>, ExecError> {
        dry_run(self.plan, self.instance_root)
    }

    /// Applies the plan, reporting through `on_event` and recording through
    /// `journal`.
    ///
    /// The last event is always terminal: [`phase::DONE`] on success,
    /// [`phase::FAILED`] on the first error. A failure stops the run; files
    /// already committed stay committed, because re-copying them is safe and
    /// deleting them would not be.
    ///
    /// The journal is a callback and not a `&Journal`, for the reason
    /// `downloader::execute_batch` gives: the handle must not be held for the
    /// length of an install, or every other command that touches the journal
    /// waits for the last byte. A journal that cannot be written stops the
    /// apply — a file nobody has a record of is a file no later run can
    /// reason about.
    ///
    /// `Send` on the callback is a consequence of the downloader wanting a
    /// `Send` progress closure; the IPC layer's channel is `Send` anyway.
    pub async fn apply(
        &self,
        journal: &mut (dyn FnMut(JournalOp) -> Result<OperationRecord, JournalError> + Send),
        on_event: &mut (dyn FnMut(&PackEvent) + Send),
    ) -> Result<ApplyReport, ExecError> {
        // A staging tree from a killed run is never resumed: the decision is
        // that `.mm-staging` only exists while an apply is running. Removing
        // it first also makes the terminal event below mean "no scratch files
        // anywhere", not just "no scratch files we happened to create".
        //
        // Unless the caller extracted the pack into that same tree: then this
        // directory holds the overrides this plan is about to copy. It is
        // still removed at the end, when nothing needs it any more.
        if !self
            .staging_root
            .starts_with(self.instance_root.join(STAGING_DIR))
        {
            self.remove_staging().await;
        }
        let mut report = ApplyReport::default();
        let mut writer = JournalWriter::new(journal);
        let outcome = self.apply_ops(&mut report, &mut writer, on_event).await;
        // The record is closed on both ordinary paths: an apply that stopped
        // half way is a `failed` operation, not one that never happened. A
        // record left `running` is offered again on every launch, forever.
        //
        // A journal that could not be written is the exception: the record is
        // what says what happened, and a record we cannot write is a record
        // we must not pretend to have closed. The operation stays unfinished,
        // which is exactly what a crash leaves behind — and a crash is what a
        // broken journal amounts to.
        let closed = match &outcome {
            Ok(()) => writer.emit(JournalOp::Operation(OperationState::Completed)),
            Err(ExecError::Journal(_)) => Ok(()),
            Err(_) => writer.emit(JournalOp::Operation(OperationState::Failed)),
        };
        // Runs on both paths, before the terminal event: a failed apply must
        // not leave its scratch directory visible to the instance scan either.
        self.remove_staging().await;
        match outcome {
            Ok(()) => match closed {
                Ok(()) => {
                    on_event(&PackEvent::operation(phase::DONE));
                    Ok(report)
                }
                // The files landed but the record did not close. That is
                // worth failing over: the operation stays unfinished, and an
                // unfinished record is a thing the user is offered.
                Err(e) => {
                    on_event(&PackEvent::operation(phase::FAILED));
                    Err(e)
                }
            },
            Err(e) => {
                on_event(&PackEvent::operation(phase::FAILED));
                Err(e)
            }
        }
    }

    /// Deletes `<instance_root>/.mm-staging` with bounded retries.
    ///
    /// shortcut: this is best-effort against a handle another process holds
    /// open. If every attempt fails, the directory stays and the next apply
    /// removes it first, so it cannot accumulate or feed a later install.
    /// Upgrade path: an explicit error or a cleanup queue in the journal if a
    /// real case ever shows a permanently locked file.
    async fn remove_staging(&self) {
        let staging = self.instance_root.join(STAGING_DIR);
        for attempt in 1..=STAGING_REMOVE_ATTEMPTS {
            match std::fs::remove_dir_all(&staging) {
                Ok(()) => return,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return,
                Err(_) if attempt < STAGING_REMOVE_ATTEMPTS => {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        STAGING_REMOVE_WAIT_MS * u64::from(attempt),
                    ))
                    .await;
                }
                Err(e) => {
                    tracing::warn!(
                        path = %staging.display(),
                        error = %e,
                        "could not remove the pack staging directory"
                    );
                    return;
                }
            }
        }
    }

    async fn apply_ops(
        &self,
        report: &mut ApplyReport,
        writer: &mut JournalWriter<'_>,
        on_event: &mut (dyn FnMut(&PackEvent) + Send),
    ) -> Result<(), ExecError> {
        // W1: the record exists before the first byte moves. A crash with no
        // record leaves files on disk that nothing claims, and that is the
        // direction this whole path exists to remove (AGENTS.md §5.4).
        let items = self.plan_items()?;
        writer.emit(JournalOp::PackStart {
            target: self.instance_root.to_string_lossy().into_owned(),
            items,
            pack: self.identity.to_meta(),
        })?;
        // W2: the apply is under way.
        writer.emit(JournalOp::Operation(OperationState::Running))?;

        for op in &self.plan.ops {
            if let InstallOp::CopyOverride {
                safe_path,
                source_layer,
            } = op
            {
                if let Err(e) = self
                    .apply_override(safe_path, source_layer, report, writer, on_event)
                    .await
                {
                    self.record_failure(writer, safe_path, &e);
                    return Err(e);
                }
            }
        }

        let downloads: Vec<&InstallOp> = self
            .plan
            .ops
            .iter()
            .filter(|op| matches!(op, InstallOp::Download { .. }))
            .collect();
        if downloads.is_empty() {
            writer.flush()?;
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
                if let Err(e) = self
                    .apply_download(safe_path, sha512, *size, &urls, report, writer, on_event)
                    .await
                {
                    self.record_failure(writer, safe_path, &e);
                    return Err(e);
                }
            }
        }
        // The last file's ending is still owed: the record says it landed
        // only now, in the same write that would have carried it if another
        // file had followed.
        writer.flush()?;
        Ok(())
    }

    /// Records the item that failed, and says so in the record.
    ///
    /// A journal that cannot be written is not recorded at all: the record is
    /// what broke, and writing a state into it now would be a state nobody
    /// can vouch for. The record is left as a crash would leave it.
    ///
    /// The journal write failing here does not replace the original error:
    /// the original error is what actually went wrong, and the record stays
    /// unfinished either way — which is the truth about it.
    fn record_failure(&self, writer: &mut JournalWriter<'_>, safe_path: &str, error: &ExecError) {
        if matches!(error, ExecError::Journal(_)) {
            return;
        }
        if let Err(e) = writer.fail_current(safe_path) {
            tracing::warn!(
                error = %e,
                "could not record the failed pack item in the journal"
            );
        }
    }

    /// The record's items: one per destination the plan names.
    ///
    /// Every item carries the `.part` this apply owns and the destination it
    /// lands at, and both are absolute: they are the proof that a `.part` on
    /// the disk is ours, and a relative path would only be true for the
    /// working directory of the process that wrote it.
    fn plan_items(&self) -> Result<Vec<OperationItem>, ExecError> {
        let mut items = Vec::with_capacity(self.plan.ops.len());
        let mut seen: HashSet<&str> = HashSet::new();
        for op in &self.plan.ops {
            let safe_path = match op {
                InstallOp::CopyOverride { safe_path, .. } => safe_path,
                InstallOp::Download { safe_path, .. } => safe_path,
            };
            // The same gate the write itself uses, so an unsafe destination
            // is refused before the record is created rather than halfway
            // through an install.
            let rel = checked_relative(safe_path)?;
            if !seen.insert(safe_path) {
                // The planner collapses duplicate destinations, so this can
                // only be a plan built by hand. Two items with one id would
                // make the record unreadable, which is worse than refusing.
                return Err(ExecError::Internal("the plan names one destination twice"));
            }
            let final_path = self.instance_root.join(&rel);
            let partial = part_path(&final_path);
            items.push(pack_item(
                safe_path,
                &self.identity.name,
                &partial.to_string_lossy(),
                &final_path.to_string_lossy(),
            ));
        }
        Ok(items)
    }

    async fn apply_override(
        &self,
        safe_path: &str,
        source_layer: &str,
        report: &mut ApplyReport,
        writer: &mut JournalWriter<'_>,
        on_event: &mut (dyn FnMut(&PackEvent) + Send),
    ) -> Result<(), ExecError> {
        let rel = checked_relative(safe_path)?;
        let dest = destination(self.instance_root, &rel)?;
        reject_link_ancestors(self.instance_root, &dest)?;
        let (layer, src) = self.override_source(&rel, source_layer)?;

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err("create_dir", parent, e))?;
        }
        on_event(&PackEvent::file(phase::COPYING, &rel));
        // Two-phase commit: the bytes land in `<dest>.part`, are checked
        // there, and only then are renamed into place. The file that becomes
        // the instance's file is the file that was checked (AGENTS.md §5.5).
        let mut reader =
            HashingReader::new(std::fs::File::open(&src).map_err(|e| io_err("open", &src, e))?);
        let bytes = write_part(&dest, &mut reader)?;
        let expected = reader.finish();
        let part = part_path(&dest);
        on_event(&PackEvent::file(phase::VERIFYING, &rel));
        let actual = sha512_of(&part).await?;
        if actual != expected {
            // Our own part, and it is wrong: it must not survive, or the next
            // run would refuse the destination because of it.
            let _ = std::fs::remove_file(&part);
            return Err(ExecError::HashMismatch {
                path: rel,
                expected,
                actual,
            });
        }
        // The record says the rename is pending before it happens, so a crash
        // in this window is visible in the record and not only on the disk.
        writer.at_commit_point(safe_path)?;
        on_event(&PackEvent::file(phase::COMMITTING, &rel));
        commit_part(&part, &dest)?;
        writer.after_rename(safe_path);
        report.written.push(AppliedFile {
            safe_path: rel.clone(),
            bytes,
            source: AppliedSource::Override { layer },
        });
        report.bytes += bytes;
        on_event(&PackEvent::file(phase::WRITTEN, &rel));
        Ok(())
    }

    /// Reads the copy from the layer the plan named.
    ///
    /// The plan chose the winner (the format's layer order); the executor must
    /// not choose again. It still validates the layer name and requires the
    /// file to exist, so a hand-built plan cannot point the read outside the
    /// staging tree.
    fn override_source(
        &self,
        rel: &str,
        source_layer: &str,
    ) -> Result<(String, PathBuf), ExecError> {
        let layer = source_layer.strip_suffix('/').unwrap_or(source_layer);
        let layer = safe_member_name(layer).map_err(|reason| ExecError::UnsafePath {
            path: source_layer.to_string(),
            reason,
        })?;
        let src = self.staging_root.join(&layer).join(rel);
        if !src.is_file() {
            return Err(ExecError::MissingOverride {
                path: rel.to_string(),
            });
        }
        Ok((layer, src))
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
        writer: &mut JournalWriter<'_>,
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
            writer.skipped(safe_path);
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
        // A pre-planted symlink or junction here would send every staged byte
        // outside the instance. `apply` removed the directory first, so what
        // exists now is either nothing or something that appeared in between.
        if is_link_or_reparse(&staging) {
            return Err(ExecError::Escape(staging));
        }
        std::fs::create_dir_all(&staging).map_err(|e| io_err("create_dir", &staging, e))?;
        let staged = staging_target(&staging, &want, &dest)?;

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
                    // Two-phase commit into the instance: the bytes land in
                    // `<dest>.part`, the hash is checked *there*, and only
                    // then is the part renamed into place. The file that
                    // becomes the instance's file is the file that was
                    // checked — the staged copy is never trusted to still
                    // hold what it held a moment ago (AGENTS.md §5.5).
                    let mut f = std::fs::File::open(&outcome.path)
                        .map_err(|e| io_err("open", &outcome.path, e))?;
                    let bytes = write_part(&dest, &mut f)?;
                    let part = part_path(&dest);
                    on_event(&PackEvent::file(phase::VERIFYING, &rel));
                    let actual = sha512_of(&part).await?;
                    if actual != want {
                        // Our own part, and it is wrong: it must not survive.
                        let _ = std::fs::remove_file(&part);
                        let _ = std::fs::remove_file(&outcome.path);
                        last_error = Some(ExecError::HashMismatch {
                            path: rel.clone(),
                            expected: want.clone(),
                            actual,
                        });
                        continue;
                    }
                    writer.at_commit_point(safe_path)?;
                    on_event(&PackEvent::file(phase::COMMITTING, &rel));
                    commit_part(&part, &dest)?;
                    writer.after_rename(safe_path);
                    let _ = std::fs::remove_file(&outcome.path);
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

        // Whatever is left in staging (a hash rejection deletes its `.part`,
        // a transport failure may keep one) is removed by `apply`'s cleanup,
        // which runs on success and failure alike. There is no resume across
        // applies by decision, so an early `remove_dir` would only race the
        // cleanup without buying anything.
        Err(last_error.unwrap_or(ExecError::NoDownloadSource {
            path: rel,
            sha512: want,
        }))
    }
}

/// The journal's side of one apply.
///
/// It exists for one reason: a record is a single document, so every write
/// rewrites the whole file, and the file grows with the number of items. A
/// pack apply that moved four states per file would rewrite the record four
/// times per file, so the bytes written grow with the *square* of the file
/// count — the volume measurement in the tests measures exactly that. So the
/// file's whole chain is written in one call, and the previous file's ending
/// rides in the same call: one write per file, not four.
struct JournalWriter<'a> {
    write: &'a mut (dyn FnMut(JournalOp) -> Result<OperationRecord, JournalError> + Send),
    /// The item whose ending is owed to the record.
    owed: Option<(String, ItemState)>,
}

impl<'a> JournalWriter<'a> {
    fn new(
        write: &'a mut (dyn FnMut(JournalOp) -> Result<OperationRecord, JournalError> + Send),
    ) -> Self {
        Self { write, owed: None }
    }

    fn emit(&mut self, op: JournalOp) -> Result<(), ExecError> {
        (self.write)(op).map(|_| ()).map_err(ExecError::from)
    }

    /// The commit point: the item's chain, and the previous item's ending.
    ///
    /// The chain is `queued → downloading → verifying → committing`: the
    /// bytes are in flight, then checked, then the rename is pending. The
    /// three steps are walked in one write, so what the record shows is
    /// `committing` — "the part is verified and the rename has not happened
    /// yet" — and never a state that is not true of the disk.
    ///
    /// The previous item's ending rides along because its file is already on
    /// disk: the record would otherwise be a file behind, and the write it
    /// saves is the difference between one write per file and two.
    fn at_commit_point(&mut self, item_id: &str) -> Result<(), ExecError> {
        let mut transitions = Vec::with_capacity(4);
        if let Some((prev, state)) = self.owed.take() {
            transitions.push((prev, state));
        }
        let id = item_id.to_string();
        transitions.push((id.clone(), ItemState::Downloading));
        transitions.push((id.clone(), ItemState::Verifying));
        transitions.push((id, ItemState::Committing));
        self.emit(JournalOp::Items { transitions })
    }

    /// The file landed; the record says so in the next write.
    fn after_rename(&mut self, item_id: &str) {
        self.owed = Some((item_id.to_string(), ItemState::Completed));
    }

    /// The destination already held the right bytes.
    fn skipped(&mut self, item_id: &str) {
        self.owed = Some((item_id.to_string(), ItemState::Skipped));
    }

    /// The item failed.
    fn fail_current(&mut self, item_id: &str) -> Result<(), ExecError> {
        let mut transitions = Vec::with_capacity(2);
        if let Some((prev, state)) = self.owed.take() {
            transitions.push((prev, state));
        }
        transitions.push((item_id.to_string(), ItemState::Failed));
        self.emit(JournalOp::Items { transitions })
    }

    /// Writes the last ending, if any.
    fn flush(&mut self) -> Result<(), ExecError> {
        if let Some((id, state)) = self.owed.take() {
            self.emit(JournalOp::Items {
                transitions: vec![(id, state)],
            })?;
        }
        Ok(())
    }
}

/// A reader that hashes what it hands over.
///
/// The source of an override is read once: the copy and the reference hash
/// come out of the same pass, so the two can never describe different bytes.
struct HashingReader<R> {
    inner: R,
    hasher: sha2::Sha512,
}

impl<R: Read> HashingReader<R> {
    fn new(inner: R) -> Self {
        use sha2::Digest as _;
        Self {
            inner,
            hasher: sha2::Sha512::new(),
        }
    }

    /// The hash of everything that was read, lowercase hex.
    fn finish(self) -> String {
        use sha2::Digest as _;
        self.hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use sha2::Digest as _;
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
        }
        Ok(n)
    }
}

/// F-076: the `.part` files an interrupted apply of THIS pack left behind.
///
/// Only a record that proves the file is ours may have its `.part` removed.
/// The proof is the record: its items name the exact `.part` this apply
/// wrote, and its pack block names the archive. A `.part` at any other path
/// is somebody else's file and stays untouched — removing it would destroy a
/// file this apply never wrote, and the refusal that follows is the honest
/// answer for a file we cannot attribute.
pub fn interrupted_parts(
    journal: &Journal,
    identity: &PackIdentity,
    instance_root: &Path,
) -> Vec<PathBuf> {
    journal
        .discover()
        .unfinished()
        .filter(|record| record.kind == OperationKind::MrpackInstall)
        .filter(|record| {
            record.pack.as_ref().is_some_and(|pack| {
                pack.archive_sha512
                    .eq_ignore_ascii_case(&identity.archive_sha512)
            })
        })
        .filter(|record| same_path(Path::new(&record.target_path), instance_root))
        .flat_map(|record| {
            record
                .items
                .iter()
                .map(|item| PathBuf::from(&item.partial_path))
        })
        .filter(|path| !path.as_os_str().is_empty())
        .collect()
}

/// F-076: removes the `.part` files an interrupted commit of this pack left.
///
/// The wedge this clears: `write_part` refuses a `.part` it cannot
/// attribute, so a `.part` left behind by an interrupted commit of the same
/// pack makes that pack permanently uninstallable — every retry meets the
/// same refusal, and the instance stays half-applied forever. The journal is
/// the only thing that can tell our part from a foreign one, so only the
/// paths a record names are removed, and only inside the instance.
pub fn clear_interrupted_parts(
    journal: &Journal,
    identity: &PackIdentity,
    instance_root: &Path,
) -> usize {
    let mut removed = 0usize;
    for part in interrupted_parts(journal, identity, instance_root) {
        // Defence in depth: the record is ours, but a hand-edited record
        // could name a path outside the instance, and this deletes.
        if !part.starts_with(instance_root) {
            tracing::warn!(
                path = %part.display(),
                "the journal names a part outside the instance; leaving it alone"
            );
            continue;
        }
        match std::fs::remove_file(&part) {
            Ok(()) => removed += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                // Best effort: the apply that follows refuses the
                // destination with the honest `StalePart` error, which says
                // more than a cleanup failure would.
                tracing::warn!(
                    path = %part.display(),
                    error = %e,
                    "could not remove the part file an interrupted commit left"
                );
            }
        }
    }
    removed
}

/// Whether two paths name the same place, even when they spell it
/// differently.
fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
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
    // Same reason as the name check: the planner refused these, but the plan
    // is data and may have been built elsewhere.
    if reserved_destination(&normalized).is_some() {
        return Err(ExecError::ReservedPath {
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

/// Sibling-named staging file, built only from hex characters of the hash.
///
/// The name comes from two sources and both are tamed: the SHA-512 is an
/// index-controlled string, so only `[0-9a-f]` characters may contribute to
/// it (`get(..16)` on a multibyte digest used to fall back to the whole
/// string, letting `../` or `/` inside it pass through into the path), and
/// the destination's file name has no separators by construction. The
/// containment assertion mirrors `destination`: a scratch path under the
/// staging root must never resolve outside that root, whatever the inputs.
fn staging_target(staging_root: &Path, sha512: &str, dest: &Path) -> Result<PathBuf, ExecError> {
    let prefix: String = sha512
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(16)
        .collect();
    let name: String = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download")
        .chars()
        .take(120)
        .collect();
    let staged = staging_root.join(format!("{prefix}-{name}"));
    // Defence in depth: the construction above cannot escape, but a helper
    // that builds a path out of unvalidated input must still assert it.
    if !staged.starts_with(staging_root) {
        return Err(ExecError::Escape(staged));
    }
    Ok(staged)
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
