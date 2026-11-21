//! Apply tests (F-121..F-124): a real instance on a temp dir, a real
//! downloader against wiremock on loopback, and a journal in its own folder.
//!
//! Nothing here fakes the filesystem or the downloader: the promises in the
//! safety contract are promises about bytes on disk, so the tests read the
//! bytes back.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha1::Sha1;
use sha2::{Digest as _, Sha512};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::apply::{
    apply_install, check_context, download_only, plan_install, BackupChoice, ContextField,
    InstallOptions, PlanTarget, SafetyContractItem, UpdateApplyError, DOWNLOAD_ONLY_DIRNAME,
};
use super::{UpdateCandidate, UpdatePreview, UpdateStatus};
use crate::downloader::Downloader;
use crate::instances::backup::BackupSubject;
use crate::instances::lock::{read_lock, write_lock, LockEntry, LockFile, LOCK_FILE_NAME};
use crate::instances::snapshot_tree;
use crate::journal::{
    is_canonical_id, ItemState, Journal, JournalError, JournalOp, OperationKind, OperationRecord,
    OperationState,
};

const OLD_MARKER: u8 = b'o';
const NEW_MARKER: u8 = b'n';
const EDITED_MARKER: u8 = b'x';

/// A real, tiny jar: the downloader refuses anything that is not an archive,
/// so the bytes in these tests have to be a zip. The same marker length keeps
/// two builds the same size, which is what lets one test change the *content*
/// of a file without changing the size the plan recorded.
fn jar_bytes(marker: u8) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        // A fixed mtime: two calls must produce the same bytes, or a test that
        // compares "the jar before" against "the jar after" compares nothing.
        let options =
            zip::write::SimpleFileOptions::default().last_modified_time(zip::DateTime::default());
        zip.start_file("pack.mcmeta", options).expect("entry");
        std::io::Write::write_all(&mut zip, &[marker; 64]).expect("entry bytes");
        zip.finish().expect("zip");
    }
    buffer.into_inner()
}

fn old_jar() -> Vec<u8> {
    jar_bytes(OLD_MARKER)
}

fn new_jar() -> Vec<u8> {
    jar_bytes(NEW_MARKER)
}

fn edited_jar() -> Vec<u8> {
    jar_bytes(EDITED_MARKER)
}

fn sha1_hex(bytes: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn sha512_hex(bytes: &[u8]) -> String {
    let mut h = Sha512::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn temp_root() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/test-tmp");
    std::fs::create_dir_all(&base).expect("test root");
    tempfile::Builder::new()
        .prefix("mm-update-")
        .tempdir_in(&base)
        .expect("temp dir")
}

struct Fixture {
    dir: tempfile::TempDir,
    root: PathBuf,
    mods: PathBuf,
    server: MockServer,
    journal: tempfile::TempDir,
}

impl Fixture {
    fn target(&self) -> PlanTarget {
        PlanTarget {
            instance_root: self.root.clone(),
            game_version: "1.20.1".to_string(),
            loader: "fabric".to_string(),
        }
    }

    fn subject(&self) -> BackupSubject {
        BackupSubject {
            instance_id: "inst-1".to_string(),
            name: "Test".to_string(),
            minecraft_version: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
        }
    }

    fn lock_path(&self) -> PathBuf {
        self.root.join(LOCK_FILE_NAME)
    }

    fn preview(&self, url: &str) -> UpdatePreview {
        UpdatePreview {
            checked: 1,
            candidates: vec![UpdateCandidate {
                filename: "old.jar".to_string(),
                provider: "modrinth".to_string(),
                project_id: "p1".to_string(),
                current_version_id: "v1".to_string(),
                current_sha1: sha1_hex(&old_jar()),
                current_sha512: sha512_hex(&old_jar()),
                latest_version_id: Some("v2".to_string()),
                latest_version_number: Some("2.0".to_string()),
                latest_filename: Some("new.jar".to_string()),
                latest_sha1: Some(sha1_hex(&new_jar())),
                latest_sha512: Some(sha512_hex(&new_jar())),
                latest_url: Some(url.to_string()),
                latest_size: Some(new_jar().len() as u64),
                latest_game_versions: vec!["1.20.1".to_string()],
                status: UpdateStatus::Available,
            }],
            warnings: Vec::new(),
        }
    }

    fn preview_dto(&self) -> UpdatePreview {
        self.preview(&format!("{}/new.jar", self.server.uri()))
    }

    fn write_lock(&self) {
        write_lock(
            &self.lock_path(),
            &LockFile {
                entries: vec![LockEntry {
                    provider: "modrinth".to_string(),
                    project_id: "p1".to_string(),
                    slug: "mod-one".to_string(),
                    version_id: "v1".to_string(),
                    filename: "old.jar".to_string(),
                    sha1: sha1_hex(&old_jar()),
                    sha512: sha512_hex(&old_jar()),
                    game_versions: vec!["1.20.1".to_string()],
                }],
            },
        )
        .expect("lock");
    }
}

async fn fixture() -> Fixture {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(new_jar()))
        .mount(&server)
        .await;
    let dir = temp_root();
    let root = dir.path().to_path_buf();
    let mods = root.join("mods");
    std::fs::create_dir_all(&mods).expect("mods");
    std::fs::write(mods.join("old.jar"), old_jar()).expect("old jar");
    let fixture = Fixture {
        dir,
        root,
        mods,
        server,
        journal: temp_root(),
    };
    fixture.write_lock();
    fixture
}

fn downloader() -> Downloader {
    Downloader::new(1).expect("download client")
}

/// A journal writer shaped like the app's: one lock per write, never held.
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
        JournalOp::PackStart { .. } => Err(JournalError::ItemMissing("not a pack apply".into())),
    }
}

/// `mods/` exactly as it is now: names, sizes and modification times.
fn mods_snapshot(mods: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    snapshot_tree(mods)
}

fn only_plan(fixture: &Fixture) -> super::apply::UpdateInstallPlan {
    let preview = fixture.preview_dto();
    plan_install(&fixture.root, "1.20.1", "fabric", &preview, &[]).expect("plan")
}

// --- F-121: re-validation before the write ---------------------------------

/// F-121: the lock moved between the preview and the confirm, so the plan is
/// about a state that no longer exists. Nothing may be written.
#[tokio::test]
async fn spec_f121_lock_changed_after_preview_aborts_before_any_write() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    let before = mods_snapshot(&fixture.mods);

    // Another install landed between the preview and the apply: the lock's
    // bytes are not the ones the plan was built from.
    let mut lock = read_lock(&fixture.lock_path()).expect("lock");
    lock.entries.push(LockEntry {
        provider: "modrinth".to_string(),
        project_id: "p9".to_string(),
        slug: "other".to_string(),
        version_id: "v9".to_string(),
        filename: "other.jar".to_string(),
        sha1: "aa".to_string(),
        sha512: "bb".to_string(),
        game_versions: Vec::new(),
    });
    write_lock(
        &fixture.lock_path(),
        &LockFile {
            entries: lock.entries,
        },
    )
    .expect("rewrite lock");

    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateInstall);
    let options = InstallOptions {
        target: &target,
        subject: fixture.subject(),
        backup: BackupChoice::no_confirmed(),
        downloader: &downloader(),
        allow_insecure_loopback: true,
    };

    let error = apply_install(&plan, &options, &mut write)
        .await
        .expect_err("a moved lock must refuse the plan");
    assert!(
        matches!(
            error,
            UpdateApplyError::ContextChanged {
                field: ContextField::Lock
            }
        ),
        "got {error:?}"
    );
    assert_eq!(
        mods_snapshot(&fixture.mods),
        before,
        "a refused plan writes nothing"
    );
    assert!(
        read_lock(&fixture.lock_path())
            .expect("lock")
            .entries
            .iter()
            .any(|e| e.project_id == "p1" && e.version_id == "v1"),
        "the lock the user had is untouched"
    );
}

/// F-121: the target the plan was built for is not the target being written.
#[tokio::test]
async fn spec_f121_a_different_target_folder_aborts_before_any_write() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    let other = fixture.root.join("other-instance");
    std::fs::create_dir_all(other.join("mods")).expect("other mods");
    let target = PlanTarget {
        instance_root: other,
        game_version: "1.20.1".to_string(),
        loader: "fabric".to_string(),
    };

    let error = check_context(&plan, &target).expect_err("a different folder is a different plan");
    assert!(
        matches!(
            error,
            UpdateApplyError::ContextChanged {
                field: ContextField::Target
            }
        ),
        "got {error:?}"
    );
}

/// F-121: building the plan is a read. `--dry-run` therefore has nothing left
/// to do, and the instance is byte-identical afterwards.
#[tokio::test]
async fn spec_f121_building_the_plan_writes_nothing() {
    let fixture = fixture().await;
    let before = mods_snapshot(&fixture.root);
    let lock_before = std::fs::read(fixture.lock_path()).expect("lock bytes");
    let _plan = only_plan(&fixture);
    assert_eq!(mods_snapshot(&fixture.root), before);
    assert_eq!(
        std::fs::read(fixture.lock_path()).expect("lock bytes"),
        lock_before
    );
    assert!(!fixture.root.join(DOWNLOAD_ONLY_DIRNAME).exists());
    let _ = &fixture.dir;
}

// --- F-122: the five-item safety contract ---------------------------------

/// The contract is data, not prose: every item names a guard, and the tests
/// below each prove one guard by its bytes on disk.
#[test]
fn spec_f122_contract_lists_five_items_each_with_its_own_guard() {
    let items = SafetyContractItem::all();
    assert_eq!(items.len(), 5, "the old app promised five things");
    let mut guards: Vec<_> = items.iter().map(|i| i.guard).collect();
    guards.sort_by_key(|g| format!("{g:?}"));
    guards.dedup();
    assert_eq!(guards.len(), 5, "two promises cannot share one guard");
    // The guard is a path into this module; a promise naming a function that
    // does not exist is the same lie as one naming nothing.
    let source = include_str!("apply.rs");
    for item in items {
        assert!(
            item.key.starts_with("updates.contract."),
            "{}: an i18n key, so both locales show the same promise",
            item.id
        );
        assert!(!item.text.is_empty(), "{}: the CLI needs the copy", item.id);
        let implemented_by = item.guard.implemented_by();
        let name = implemented_by.rsplit("::").next().unwrap_or_default();
        assert!(
            !name.is_empty() && source.contains(&format!("fn {name}(")),
            "{}: claims '{implemented_by}', which is not a function in this module",
            item.id
        );
    }
}

/// Contract 1: the replaced jar is moved into a timestamped backup folder.
#[tokio::test]
async fn spec_f122_replaced_jar_moves_into_a_timestamped_backup_folder() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateInstall);
    let options = InstallOptions {
        target: &target,
        subject: fixture.subject(),
        backup: BackupChoice::no_confirmed(),
        downloader: &downloader(),
        allow_insecure_loopback: true,
    };
    let report = apply_install(&plan, &options, &mut write)
        .await
        .expect("install");
    assert_eq!(report.installed.len(), 1);
    let dir = &report.replaced_backup_dir;
    assert!(
        dir.starts_with(fixture.root.join(".modrinth_bulk_downloader_backup")),
        "the replaced file lives in the app's own dated folder, got {}",
        dir.display()
    );
    let kept = std::fs::read(dir.join("old.jar")).expect("the old jar is kept");
    assert_eq!(
        kept,
        old_jar(),
        "the backup holds the bytes that were replaced"
    );
    assert_eq!(
        std::fs::read(fixture.mods.join("new.jar")).expect("new"),
        new_jar()
    );
    assert!(
        !fixture.mods.join("old.jar").exists(),
        "the old file was moved, not copied in place"
    );
}

/// Contract 2: nothing is permanently deleted — both the old and the new
/// bytes exist after a successful install.
#[tokio::test]
async fn spec_f122_no_preexisting_file_is_deleted_by_an_install() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateInstall);
    let options = InstallOptions {
        target: &target,
        subject: fixture.subject(),
        backup: BackupChoice::no_confirmed(),
        downloader: &downloader(),
        allow_insecure_loopback: true,
    };
    apply_install(&plan, &options, &mut write)
        .await
        .expect("install");
    let backed_up: Vec<Vec<u8>> = std::fs::read_dir(report_dir(&fixture))
        .expect("backup dir")
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| std::fs::read(e.path()).expect("bytes"))
        .collect();
    assert!(
        backed_up
            .iter()
            .any(|b| b.as_slice() == old_jar().as_slice()),
        "the replaced bytes still exist somewhere"
    );
    assert_eq!(
        std::fs::read(fixture.mods.join("new.jar")).expect("new"),
        new_jar()
    );
}

/// The only place an install writes besides `mods/` and its own backup folder.
fn report_dir(fixture: &Fixture) -> PathBuf {
    let root = fixture.root.join(".modrinth_bulk_downloader_backup");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("backup root")
        .flatten()
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs.pop().expect("one dated folder")
}

/// Contract 3: a jar nobody planned is not read, moved or rewritten.
#[tokio::test]
async fn spec_f122_unknown_files_in_mods_are_not_touched() {
    let fixture = fixture().await;
    let unknown = fixture.mods.join("handmade.jar");
    std::fs::write(&unknown, b"my own build").expect("unknown jar");
    let before = std::fs::metadata(&unknown).expect("meta");
    let plan = only_plan(&fixture);
    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateInstall);
    let options = InstallOptions {
        target: &target,
        subject: fixture.subject(),
        backup: BackupChoice::no_confirmed(),
        downloader: &downloader(),
        allow_insecure_loopback: true,
    };
    apply_install(&plan, &options, &mut write)
        .await
        .expect("install");
    let after = std::fs::metadata(&unknown).expect("the unknown jar is still there");
    assert_eq!(std::fs::read(&unknown).expect("bytes"), b"my own build");
    assert_eq!(after.len(), before.len());
    assert_eq!(
        after.modified().expect("mtime"),
        before.modified().expect("mtime"),
        "an unplanned file must not even be rewritten with identical bytes"
    );
}

/// Contract 4: a local jar that changed since the preview is skipped, and the
/// rest of the batch still lands.
#[tokio::test]
async fn spec_f122_local_file_changed_since_preview_is_skipped() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    // Same length, different bytes: the folder facts the plan recorded still
    // match, so only the recorded hash can tell.
    std::fs::write(fixture.mods.join("old.jar"), edited_jar()).expect("rewrite");

    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateInstall);
    let options = InstallOptions {
        target: &target,
        subject: fixture.subject(),
        backup: BackupChoice::no_confirmed(),
        downloader: &downloader(),
        allow_insecure_loopback: true,
    };
    let report = apply_install(&plan, &options, &mut write)
        .await
        .expect("install");
    assert!(report.installed.is_empty());
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(
        report.skipped[0].reason,
        super::apply::SkipReason::ChangedSincePreview
    );
    assert_eq!(
        std::fs::read(fixture.mods.join("old.jar")).expect("local file"),
        edited_jar(),
        "the file the user edited is left exactly as it is"
    );
    assert!(!fixture.mods.join("new.jar").exists());
}

/// Contract 5: a file already sitting at the new name is never overwritten
/// unless it is byte-for-byte the file being installed.
#[tokio::test]
async fn spec_f122_an_unrelated_file_at_the_target_name_is_never_overwritten() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    std::fs::write(fixture.mods.join("new.jar"), edited_jar()).expect("occupant");

    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateInstall);
    let options = InstallOptions {
        target: &target,
        subject: fixture.subject(),
        backup: BackupChoice::no_confirmed(),
        downloader: &downloader(),
        allow_insecure_loopback: true,
    };
    let report = apply_install(&plan, &options, &mut write)
        .await
        .expect("install");
    assert!(report.installed.is_empty());
    assert_eq!(
        report.skipped[0].reason,
        super::apply::SkipReason::TargetOccupied
    );
    assert_eq!(
        std::fs::read(fixture.mods.join("new.jar")).expect("occupant"),
        edited_jar()
    );
    assert_eq!(
        std::fs::read(fixture.mods.join("old.jar")).expect("old"),
        old_jar(),
        "nothing was moved out from under the occupant either"
    );
}

// --- F-123: the three-way backup decision ---------------------------------

/// Yes: the full-profile backup of F-113 runs before the install, and it is
/// a real backup (dated folder + manifest), not a log line.
#[tokio::test]
async fn spec_f123_backup_yes_runs_the_profile_backup_first() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateInstall);
    let options = InstallOptions {
        target: &target,
        subject: fixture.subject(),
        backup: BackupChoice::yes(),
        downloader: &downloader(),
        allow_insecure_loopback: true,
    };
    let report = apply_install(&plan, &options, &mut write)
        .await
        .expect("install");
    let run = report.profile_backup.expect("Yes must produce a backup");
    assert!(run.backup_dir.is_dir());
    assert!(
        run.manifest_path.is_file(),
        "a folder without a manifest is not a backup"
    );
    assert!(
        run.backup_dir.join("mods").join("old.jar").is_file(),
        "the pre-install mods/ is inside the backup"
    );
    assert_eq!(
        std::fs::read(run.backup_dir.join("mods").join("old.jar")).expect("copy"),
        old_jar(),
        "the backup kept the file as it was before the install"
    );
    assert_eq!(
        std::fs::read(fixture.mods.join("new.jar")).expect("new"),
        new_jar()
    );
}

/// No, without the second confirmation: refused, nothing written.
#[tokio::test]
async fn spec_f123_backup_no_without_the_second_confirmation_writes_nothing() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    let before = mods_snapshot(&fixture.root);
    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateInstall);
    let options = InstallOptions {
        target: &target,
        subject: fixture.subject(),
        backup: BackupChoice::no(),
        downloader: &downloader(),
        allow_insecure_loopback: true,
    };
    let error = apply_install(&plan, &options, &mut write)
        .await
        .expect_err("No is not a confirmation");
    assert!(
        matches!(error, UpdateApplyError::BackupNotConfirmed),
        "got {error:?}"
    );
    assert_eq!(mods_snapshot(&fixture.root), before);
    assert!(!fixture
        .root
        .join(".modrinth_bulk_downloader_profile_backups")
        .exists());
    assert_eq!(
        journal.discover().records().count(),
        0,
        "no operation was started"
    );
}

/// Cancel: the operation stops with no write at all.
#[tokio::test]
async fn spec_f123_backup_cancel_aborts_with_no_write() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    let before = mods_snapshot(&fixture.root);
    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateInstall);
    let options = InstallOptions {
        target: &target,
        subject: fixture.subject(),
        backup: BackupChoice::cancel(),
        downloader: &downloader(),
        allow_insecure_loopback: true,
    };
    let error = apply_install(&plan, &options, &mut write)
        .await
        .expect_err("Cancel stops the run");
    assert!(
        matches!(error, UpdateApplyError::Cancelled),
        "got {error:?}"
    );
    assert_eq!(mods_snapshot(&fixture.root), before);
    assert_eq!(journal.discover().records().count(), 0);
}

/// No, confirmed: the install runs, and the only copy kept is the replaced
/// jar — the user was told exactly that.
#[tokio::test]
async fn spec_f123_backup_no_confirmed_installs_without_a_profile_copy() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateInstall);
    let options = InstallOptions {
        target: &target,
        subject: fixture.subject(),
        backup: BackupChoice::no_confirmed(),
        downloader: &downloader(),
        allow_insecure_loopback: true,
    };
    let report = apply_install(&plan, &options, &mut write)
        .await
        .expect("install");
    assert!(report.profile_backup.is_none());
    assert!(!fixture
        .root
        .join(".modrinth_bulk_downloader_profile_backups")
        .exists());
    assert!(
        report.replaced_backup_dir.is_dir(),
        "the replaced jar is still kept"
    );
    assert_eq!(
        std::fs::read(fixture.mods.join("new.jar")).expect("new"),
        new_jar()
    );
}

/// Every branch is recorded: the operation ends completed with one item per
/// planned file.
#[tokio::test]
async fn spec_f123_the_install_is_journalled_item_by_item() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateInstall);
    let options = InstallOptions {
        target: &target,
        subject: fixture.subject(),
        backup: BackupChoice::no_confirmed(),
        downloader: &downloader(),
        allow_insecure_loopback: true,
    };
    let report = apply_install(&plan, &options, &mut write)
        .await
        .expect("install");
    assert!(is_canonical_id(&report.operation_id));
    let record = journal.read(&report.operation_id).expect("record");
    assert_eq!(record.kind, OperationKind::UpdateInstall);
    assert_eq!(record.state, OperationState::Completed);
    assert_eq!(record.items.len(), 1);
    assert_eq!(record.items[0].state, ItemState::Completed);
    assert!(record.items[0].final_path.ends_with("new.jar"));
}

// --- F-124: download-only --------------------------------------------------

/// Download-only writes the prepared files into `modrinth-update-downloads`
/// and leaves `mods/` byte-identical.
#[tokio::test]
async fn spec_f124_download_only_leaves_mods_byte_identical() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    let before = mods_snapshot(&fixture.mods);
    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateDownload);
    let report = download_only(&plan, &target, &downloader(), true, &mut write)
        .await
        .expect("download only");
    assert_eq!(
        mods_snapshot(&fixture.mods),
        before,
        "download-only installs nothing"
    );
    assert_eq!(report.prepared.len(), 1);
    let prepared = &report.prepared[0];
    assert!(Path::new(&prepared.path).starts_with(fixture.root.join(DOWNLOAD_ONLY_DIRNAME)));
    assert_eq!(std::fs::read(&prepared.path).expect("bytes"), new_jar());
    assert!(!fixture.mods.join("new.jar").exists());
}

/// Download-only is journal-led too, so a crash leaves a record of what was
/// being prepared and where.
#[tokio::test]
async fn spec_f124_download_only_journals_the_prepared_files() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    let target = fixture.target();
    let journal = Journal::new(fixture.journal.path());
    let mut write = writer(&journal, OperationKind::UpdateDownload);
    let report = download_only(&plan, &target, &downloader(), true, &mut write)
        .await
        .expect("download only");
    let record = journal.read(&report.operation_id).expect("record");
    assert_eq!(record.kind, OperationKind::UpdateDownload);
    assert_eq!(record.state, OperationState::Completed);
    assert_eq!(record.items.len(), 1);
    assert!(
        record.items[0].final_path.starts_with(
            &fixture
                .root
                .join(DOWNLOAD_ONLY_DIRNAME)
                .to_string_lossy()
                .to_string()
        ),
        "the record names the download folder, not mods/"
    );
}

/// The journal writer is the caller's: a refused start must never be ignored.
#[tokio::test]
async fn spec_f124_a_failed_journal_start_stops_before_the_download() {
    let fixture = fixture().await;
    let plan = only_plan(&fixture);
    let target = fixture.target();
    let mut write = |_op: JournalOp| -> Result<OperationRecord, JournalError> {
        Err(JournalError::Io("journal unavailable".to_string()))
    };
    let error = download_only(&plan, &target, &downloader(), true, &mut write)
        .await
        .expect_err("no journal, no operation");
    assert!(
        matches!(error, UpdateApplyError::Journal(_)),
        "got {error:?}"
    );
    assert!(!fixture.root.join(DOWNLOAD_ONLY_DIRNAME).exists());
}
