//! `update`: newest compatible per lock pin, then swap — through the core's
//! plan/apply, never by hand here.
//!
//! This command is a thin client of [`mm_core::updates::apply`]. The safety
//! contract (F-122) is printed from core's own table, so the CLI cannot drift
//! from what the app shows; the backup question (F-123) has no default; and
//! `--download-only` (F-124) prepares files without touching `mods/`.

use super::{client, confirm_forced, instance_dir, CliError, Ctx};
use mm_core::downloader::Downloader;
use mm_core::instances::backup::BackupSubject;
use mm_core::instances::metadata::{read_mirror, PROFILE_FILE_NAME};
use mm_core::journal::{Journal, JournalError, JournalOp, OperationKind, OperationRecord};
use mm_core::updates::apply::{
    apply_install, download_only, plan_install, BackupChoice, InstallOptions, SafetyContractItem,
    UpdateApplyError, DOWNLOAD_ONLY_DIRNAME,
};
use mm_core::updates::{check_updates, UpdateStatus};
use std::path::Path;

/// The three-way answer to "create a full profile backup first?" (F-123).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupArg {
    Yes,
    No,
    Cancel,
}

/// What the flags decided, in one value: `update` already takes the context,
/// and seven positional arguments would be unreadable at the call site.
pub struct UpdateFlags {
    pub game: Option<String>,
    pub loader: Option<String>,
    pub yes: bool,
    pub dry_run: bool,
    pub download_only: bool,
    pub backup: Option<BackupArg>,
    pub confirm_no_backup: bool,
}

pub async fn update(ctx: &Ctx, flags: UpdateFlags) -> Result<(), CliError> {
    let UpdateFlags {
        game,
        loader,
        yes,
        dry_run,
        download_only: download_only_mode,
        backup,
        confirm_no_backup,
    } = flags;
    let game = game.unwrap_or_else(|| "any".to_string());
    let loader = loader.unwrap_or_else(|| "any".to_string());
    if game == "any" || loader == "any" {
        return Err(CliError::Usage(
            "update requires --game and --loader".to_string(),
        ));
    }
    let instance = instance_dir(ctx)?;
    let provider = client(ctx)?;
    let preview = check_updates(&provider, &instance, &game, &loader)
        .await
        .map_err(|e| CliError::Provider(e.to_string()))?;

    let available: Vec<Vec<String>> = preview
        .candidates
        .iter()
        .filter(|c| c.status == UpdateStatus::Available)
        .map(|c| {
            vec![
                c.project_id.clone(),
                c.current_version_id.clone(),
                c.latest_version_number.clone().unwrap_or_default(),
                c.latest_filename.clone().unwrap_or_default(),
            ]
        })
        .collect();
    if available.is_empty() {
        println!("already up to date");
        return Ok(());
    }

    let plan = match plan_install(&instance, &game, &loader, &preview, &[]) {
        Ok(plan) => plan,
        Err(UpdateApplyError::EmptyPlan) => {
            println!("already up to date");
            return Ok(());
        }
        Err(e) => return Err(apply_error(e)),
    };

    super::print_table(
        &["project", "old", "new", "file"],
        &plan
            .entries
            .iter()
            .map(|e| {
                vec![
                    e.project_id.clone(),
                    e.version_id.clone(),
                    e.version_number.clone(),
                    e.filename.clone(),
                ]
            })
            .collect::<Vec<_>>(),
    );

    // F-122: the five promises are shown before the confirmation, in both
    // modes — they are about what the run does, not only about installing.
    println!();
    println!("What this does:");
    for item in SafetyContractItem::all() {
        println!("  - {}", item.text);
    }
    println!();

    if dry_run {
        if download_only_mode {
            println!(
                "dry run: {} file(s) would be prepared in {DOWNLOAD_ONLY_DIRNAME}/",
                plan.entries.len()
            );
        } else {
            println!("dry run: {} file(s) would be replaced", plan.entries.len());
        }
        return Ok(());
    }

    if download_only_mode {
        confirm_forced(plan.entries.len(), yes)?;
        let target = plan.context.target.clone();
        let downloader = Downloader::new(3).map_err(|e| CliError::Download(e.to_string()))?;
        let journal = journal()?;
        let mut write = writer(&journal, OperationKind::UpdateDownload);
        let report = download_only(
            &plan,
            &target,
            &downloader,
            ctx.insecure_loopback,
            &mut write,
        )
        .await
        .map_err(apply_error)?;
        for file in &report.prepared {
            println!("prepared {}", file.path);
        }
        println!(
            "{} file(s) in {}",
            report.prepared.len(),
            report.directory.display()
        );
        return Ok(());
    }

    // F-123: no default. A missing answer is a usage error, not a quiet Yes.
    let choice = match backup {
        Some(BackupArg::Yes) => BackupChoice::yes(),
        Some(BackupArg::No) if confirm_no_backup => BackupChoice::no_confirmed(),
        Some(BackupArg::No) => {
            return Err(CliError::Usage(
                "refusing to install without a profile backup: pass --confirm-no-backup \
                 to accept that the replaced jars are the only copies kept"
                    .to_string(),
            ))
        }
        Some(BackupArg::Cancel) => BackupChoice::cancel(),
        None => {
            return Err(CliError::Usage(
                "update requires --backup yes|no|cancel (no default)".to_string(),
            ))
        }
    };

    confirm_forced(plan.entries.len(), yes)?;
    let target = plan.context.target.clone();
    let downloader = Downloader::new(3).map_err(|e| CliError::Download(e.to_string()))?;
    let options = InstallOptions {
        target: &target,
        subject: backup_subject(&instance),
        backup: choice,
        downloader: &downloader,
        allow_insecure_loopback: ctx.insecure_loopback,
    };
    let journal = journal()?;
    let mut write = writer(&journal, OperationKind::UpdateInstall);
    let report = apply_install(&plan, &options, &mut write)
        .await
        .map_err(apply_error)?;
    for row in &report.installed {
        println!("updated {} -> {}", row.project_id, row.version_number);
    }
    for row in &report.skipped {
        println!("skipped {}: {}", row.filename, row.message);
    }
    println!(
        "replaced files kept in {}",
        report.replaced_backup_dir.display()
    );
    if let Some(run) = &report.profile_backup {
        println!("profile backup {}", run.backup_dir.display());
    }
    Ok(())
}

/// The journal store the app itself uses, resolved the same way.
fn journal() -> Result<Journal, CliError> {
    let paths = mm_core::paths::Paths::resolve().map_err(|e| CliError::Io(e.to_string()))?;
    Ok(Journal::new(paths.operations_dir()))
}

/// One lock per write, exactly like the app: a download can take minutes and
/// must not hold the store.
fn writer<'a>(
    journal: &'a Journal,
    kind: OperationKind,
) -> impl FnMut(JournalOp) -> Result<OperationRecord, JournalError> + 'a {
    let mut id = String::new();
    move |op: JournalOp| match op {
        JournalOp::Start { target, items } => {
            let record = journal.start(kind, "", &target, items)?;
            id = record.operation_id.clone();
            Ok(record)
        }
        JournalOp::Operation(state) => journal.transition_operation(&id, state, None),
        JournalOp::Item { item_id, state } => journal.transition_item(&id, &item_id, state, None),
        JournalOp::Items { transitions } => journal.transition_items(&id, &transitions, None),
        JournalOp::PackStart { .. } => Err(JournalError::ItemMissing(
            "an update is not a pack apply".to_string(),
        )),
    }
}

/// Who a profile backup is about. The instance's own `profile.json` names it;
/// a folder ModManager did not create falls back to its directory name, and the
/// backup is still a real backup of the real bytes.
fn backup_subject(instance: &Path) -> BackupSubject {
    let fallback = instance
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "instance".to_string());
    match read_mirror(&instance.join(PROFILE_FILE_NAME)) {
        Ok(meta) => BackupSubject {
            instance_id: meta.id,
            name: meta.name,
            minecraft_version: meta.minecraft_version,
            loader: meta.loader,
        },
        Err(_) => BackupSubject {
            instance_id: fallback.clone(),
            name: fallback,
            minecraft_version: None,
            loader: None,
        },
    }
}

fn apply_error(error: UpdateApplyError) -> CliError {
    match error {
        UpdateApplyError::ContextChanged { field } => CliError::Blocked(format!(
            "the instance changed after the plan was shown ({field:?}); show it again"
        )),
        UpdateApplyError::BackupNotConfirmed => CliError::ConfirmationRequired,
        UpdateApplyError::Cancelled => CliError::Blocked("cancelled; nothing was written".into()),
        UpdateApplyError::EmptyPlan => CliError::Usage("nothing to install".into()),
        UpdateApplyError::NeedsTarget => {
            CliError::Usage("update requires a concrete Minecraft version and loader".into())
        }
        UpdateApplyError::Backup(e) => CliError::Io(format!("backup: {e}")),
        UpdateApplyError::Download(e) => CliError::Download(e.to_string()),
        UpdateApplyError::Journal(e) => CliError::Io(format!("journal: {e}")),
        UpdateApplyError::Lock(e) => CliError::Io(format!("lock: {e}")),
        UpdateApplyError::Io { .. } => CliError::Io(error.to_string()),
    }
}
