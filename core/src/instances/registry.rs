//! F-102 create a managed instance, F-103 link one, F-107 roll back.
//!
//! Plan first, execute second, dry-run is the plan (AGENTS.md §5.4).
//!
//! Write order is directories, then the `profile.json` mirror, then the
//! database row last, so a failed insert can be rolled back by removing exactly
//! what this call created.

use std::path::{Path, PathBuf};

use thiserror::Error;
use uuid::Uuid;

use crate::db::instances::{self, InstanceRow, InstanceRowError, InstanceSource};
use crate::instances::metadata::{self, MetadataError};

/// The only folders this phase creates. `mods` is required by validation and
/// `config` is the other folder the reference lists as standard; the rest
/// (saves, resourcepacks, shaderpacks) are optional by design — the reference
/// grades a missing one as a warning because launchers create them on first
/// run.
pub const STANDARD_DIRS: [&str; 2] = ["mods", "config"];

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("refusing to create in {0}: the folder already exists and is not empty")]
    RootNotEmpty(PathBuf),
    #[error("refusing to create in {0}: it already holds an instance")]
    AlreadyAnInstance(PathBuf),
    #[error("refusing to create in {0}: the path is a file")]
    RootIsFile(PathBuf),
    #[error("unsafe instance name: {0}")]
    UnsafeName(String),
    #[error(transparent)]
    Row(#[from] InstanceRowError),
    #[error(transparent)]
    Metadata(#[from] MetadataError),
    #[error(transparent)]
    Journal(#[from] crate::journal::JournalError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("rolled back {0} step(s) after a failure: {1}")]
    RolledBack(usize, String),
}

/// What `execute_create` will do. Building this performs no writes, so it is
/// also the dry run the UI shows before asking for confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePlan {
    pub root: PathBuf,
    pub name: String,
    pub minecraft_version: Option<String>,
    pub loader: Option<String>,
    pub id: String,
    pub created_at: i64,
    /// Absolute directories that will be created, in order, root first.
    pub dirs: Vec<PathBuf>,
}

impl CreatePlan {
    /// Directories below the root, relative, for display.
    pub fn relative_dirs(&self) -> Vec<&str> {
        STANDARD_DIRS.to_vec()
    }
}

/// F-106: refuse to write into a folder that already holds an instance, and
/// offer linking instead.
pub fn plan_create(
    root: &Path,
    name: &str,
    minecraft_version: Option<String>,
    loader: Option<String>,
    created_at: i64,
) -> Result<CreatePlan, RegistryError> {
    let safe = crate::safety::path::safe_member_name(name)
        .map_err(|_| RegistryError::UnsafeName(name.to_string()))?;

    if root.exists() {
        if !root.is_dir() {
            return Err(RegistryError::RootIsFile(root.to_path_buf()));
        }
        if metadata::read_mirror(&metadata::mirror_path(root)).is_ok() {
            return Err(RegistryError::AlreadyAnInstance(root.to_path_buf()));
        }
        if std::fs::read_dir(root)
            .map_err(RegistryError::Io)?
            .next()
            .is_some()
        {
            return Err(RegistryError::RootNotEmpty(root.to_path_buf()));
        }
    }

    let mut dirs = Vec::with_capacity(STANDARD_DIRS.len() + 1);
    dirs.push(root.to_path_buf());
    for d in STANDARD_DIRS {
        dirs.push(root.join(d));
    }

    Ok(CreatePlan {
        root: root.to_path_buf(),
        name: safe,
        minecraft_version,
        loader,
        id: Uuid::new_v4().to_string(),
        created_at,
        dirs,
    })
}

/// Creates the folders and the mirror, then the row — journalling first, so a
/// crash leaves evidence instead of an orphan.
///
/// Order: `Planned` record → `Running` → directories → mirror → row →
/// `Completed`. A crash therefore means one of:
/// * after the record, before any mkdir — nothing written anywhere;
/// * mid-directories — some empty folders;
/// * after the mirror, before the row — a complete `profile.json`, no row.
///
/// `crate::instances::recovery` turns those into an offer, never an action.
///
/// `link_instance` is deliberately absent: one atomic row, nothing partial to
/// reconcile, so a record would be noise in `discover()`.
pub fn execute_create(
    db: &crate::db::Db,
    journal: &crate::journal::Journal,
    plan: &CreatePlan,
) -> Result<InstanceRow, RegistryError> {
    // Journal first. If this fails the create must not start: an unjournaled
    // create is exactly the orphan the record exists to describe.
    let record = journal.start(
        crate::journal::OperationKind::InstanceCreate,
        &plan.id,
        &plan.root.to_string_lossy(),
        Vec::new(),
    )?;
    let operation_id = record.operation_id.clone();

    // Only directories this call actually created. A folder that already existed
    // is not ours to remove, and this is also what keeps the root safe:
    // `plan_create` permits a pre-existing empty root, and that root is absent
    // from `created` precisely because we did not make it.
    let mut created: Vec<PathBuf> = Vec::with_capacity(plan.dirs.len());

    let build = (|| -> Result<InstanceRow, RegistryError> {
        journal.transition_operation(
            &operation_id,
            crate::journal::OperationState::Running,
            None,
        )?;

        // Only directories this call actually created. A folder that already existed
        // is not ours to remove, and this is also what keeps the root safe:
        // `plan_create` permits a pre-existing empty root, and that root is
        // absent from `created` precisely because we did not make it.
        for dir in &plan.dirs {
            match std::fs::create_dir(dir) {
                Ok(()) => created.push(dir.clone()),
                // Already there as a folder means an earlier plan ran, so it is
                // not ours to remove on rollback. A *file* in the way is a real
                // failure and must not be swallowed.
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && dir.is_dir() => {}
                Err(e) => return Err(e.into()),
            }
        }

        let row = InstanceRow {
            id: plan.id.clone(),
            name: plan.name.clone(),
            path: plan.root.clone(),
            minecraft_root: PathBuf::new(),
            minecraft_version: plan.minecraft_version.clone(),
            loader: plan.loader.clone(),
            source: InstanceSource::Managed,
            created_at: plan.created_at,
            scanned_at: None,
        };
        metadata::write_mirror(&plan.root, &metadata::mirror_of(&row))?;
        instances::insert(db, &row)?;
        let stored = instances::get(db, &row.id)
            .map_err(RegistryError::Row)?
            .ok_or(RegistryError::Row(InstanceRowError::DuplicatePath(
                row.path,
            )))?;
        // Completed last, after the row exists: a record that says Completed
        // must never point at an instance that is not there.
        let _ = journal.transition_operation(
            &operation_id,
            crate::journal::OperationState::Completed,
            None,
        );
        Ok(stored)
    })();

    match build {
        Ok(row) => Ok(row),
        Err(err) => {
            // `remove_dir`, never `remove_dir_all`: a directory that gained
            // content we did not put there must survive. Only `created` is
            // walked, so a pre-existing folder — including a pre-existing
            // empty root — is never touched.
            let mut undone = 0usize;
            for dir in created.iter().rev() {
                if std::fs::remove_dir(dir).is_ok() {
                    undone += 1;
                }
            }
            // Failed only after the folders are actually gone, so a crash
            // during rollback leaves the record Running for recovery to see.
            let _ = journal.transition_operation(
                &operation_id,
                crate::journal::OperationState::Failed,
                None,
            );
            Err(RegistryError::RolledBack(undone, err.to_string()))
        }
    }
}

/// F-103: registers a folder the user already has. Nothing is created, moved
/// or written inside it — not even the mirror.
pub fn link_instance(
    db: &crate::db::Db,
    root: &Path,
    name: &str,
    created_at: i64,
) -> Result<InstanceRow, RegistryError> {
    let (instance_root, minecraft_root) = crate::instances::detect_instance_roots(root);
    let row = InstanceRow {
        id: Uuid::new_v4().to_string(),
        name: name.to_string(),
        path: instance_root,
        minecraft_root,
        // Detection is F-092/093, deferred; an undetected instance stays
        // unknown until the user sets it (F-101).
        minecraft_version: None,
        loader: None,
        source: InstanceSource::Manual,
        created_at,
        scanned_at: None,
    };
    instances::insert(db, &row)?;
    instances::get(db, &row.id)
        .map_err(RegistryError::Row)?
        .ok_or(RegistryError::Row(InstanceRowError::DuplicatePath(
            row.path,
        )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open, Db};

    fn plan(root: &Path) -> CreatePlan {
        plan_create(
            root,
            "Survival",
            Some("1.21.1".to_string()),
            Some("fabric".to_string()),
            100,
        )
        .expect("plan")
    }

    fn db(dir: &Path) -> Db {
        open(&dir.join("t.db")).expect("open")
    }

    fn jrnl(_dir: &Path) -> (crate::journal::Journal, tempfile::TempDir) {
        let holder = crate::test_tempdir();
        (
            crate::journal::Journal::new(holder.path().join("operations")),
            holder,
        )
    }

    fn names(root: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(root)
            .expect("read_dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn spec_f102_create_writes_dirs_mirror_and_row() {
        let dir = crate::test_tempdir();
        let root = dir.path().join("Survival");
        let p = plan(&root);

        let (j, _jh) = jrnl(dir.path());
        let row = execute_create(&db(dir.path()), &j, &p).expect("create");

        assert!(root.join("mods").is_dir());
        assert!(root.join("config").is_dir());
        assert!(metadata::mirror_path(&root).is_file());
        assert_eq!(row.source, InstanceSource::Managed);
        assert_eq!(names(&root), vec!["config", "mods", "profile.json"]);

        let listed = instances::list(&db(dir.path())).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].minecraft_version.as_deref(), Some("1.21.1"));

        let back = metadata::read_mirror(&metadata::mirror_path(&root)).expect("mirror");
        assert_eq!(back.id, row.id);
        assert_eq!(back.typed_source().expect("typed"), InstanceSource::Managed);
    }

    /// Builds a plan rooted at `wrapper`, with `profile.json` blocked by a
    /// directory so the mirror write fails *after* the folders were created — the
    /// only shape that actually exercises the rollback.
    fn blocked_plan(wrapper: &Path) -> CreatePlan {
        std::fs::create_dir_all(wrapper.join(metadata::PROFILE_FILE_NAME)).expect("block mirror");
        let mut p = plan(&wrapper.join("unused"));
        p.root = wrapper.to_path_buf();
        p.dirs = STANDARD_DIRS
            .iter()
            .fold(vec![wrapper.to_path_buf()], |mut acc, d| {
                acc.push(wrapper.join(d));
                acc
            });
        p
    }

    #[test]
    fn spec_f107_create_rolls_back_every_directory_it_made() {
        let dir = crate::test_tempdir();
        let wrapper = dir.path().join("wrapper");
        let p = blocked_plan(&wrapper);

        let (j, _jh) = jrnl(dir.path());
        let err = execute_create(&db(dir.path()), &j, &p).expect_err("mirror write must fail");
        let undone = match &err {
            RegistryError::RolledBack(n, _) => *n,
            other => panic!("expected a rollback, got {other:?}"),
        };
        assert_eq!(undone, 2, "mods and config were ours: {err:?}");
        assert!(!wrapper.join("mods").exists(), "created dir rolled back");
        assert!(!wrapper.join("config").exists(), "created dir rolled back");
        assert!(
            wrapper.is_dir(),
            "the pre-existing root is not ours to delete"
        );
        assert_eq!(
            instances::list(&db(dir.path())).expect("list").len(),
            0,
            "no row survives a failed create"
        );
    }

    #[test]
    fn spec_f107_rollback_never_removes_a_directory_it_did_not_create() {
        let dir = crate::test_tempdir();
        let wrapper = dir.path().join("wrapper");
        std::fs::create_dir_all(wrapper.join("mods")).expect("pre-existing mods");
        let p = blocked_plan(&wrapper);

        let (j, _jh) = jrnl(dir.path());
        let err = execute_create(&db(dir.path()), &j, &p).expect_err("must fail");
        assert!(
            matches!(err, RegistryError::RolledBack(1, _)),
            "only config was ours to remove, got {err:?}"
        );
        assert!(wrapper.join("mods").is_dir(), "not emptied by rollback");
        assert!(!wrapper.join("config").exists());
    }

    #[test]
    fn spec_f106_create_refuses_a_file_in_place_of_a_folder() {
        let dir = crate::test_tempdir();
        let root = dir.path().join("wrapper");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::write(root.join("config"), b"x").expect("a file, not a folder");
        // plan_create refuses the non-empty root; bypass it to reach the guard.
        let mut p = plan(&dir.path().join("elsewhere"));
        p.root = root.clone();
        p.dirs = vec![root.clone(), root.join("mods"), root.join("config")];

        let (j, _jh) = jrnl(dir.path());
        let err = execute_create(&db(dir.path()), &j, &p).expect_err("must fail");
        assert!(
            matches!(err, RegistryError::RolledBack(1, _)),
            "got {err:?}"
        );
        assert!(root.join("config").is_file(), "the user's file survives");
        assert!(!root.join("mods").exists(), "the folder we made is gone");
    }

    #[test]
    fn spec_f106_create_refuses_a_non_empty_root() {
        let dir = crate::test_tempdir();
        let root = dir.path().join("Minecraft");
        std::fs::create_dir_all(root.join("saves")).expect("user content");
        let err = plan_create(&root, "n", None, None, 1).expect_err("refuse");
        assert!(matches!(err, RegistryError::RootNotEmpty(_)), "got {err:?}");
    }

    #[test]
    fn spec_f106_create_refuses_an_existing_instance_folder() {
        let dir = crate::test_tempdir();
        let root = dir.path().join("Existing");
        let p = plan(&root);
        let (j, _jh) = jrnl(dir.path());
        execute_create(&db(dir.path()), &j, &p).expect("first");

        let err = plan_create(&root, "Other", None, None, 1).expect_err("refuse");
        assert!(
            matches!(err, RegistryError::AlreadyAnInstance(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn spec_f106_create_refuses_a_file_and_an_unsafe_name() {
        let dir = crate::test_tempdir();
        let file = dir.path().join("a.jar");
        std::fs::write(&file, b"x").expect("write");
        assert!(matches!(
            plan_create(&file, "n", None, None, 1).expect_err("file"),
            RegistryError::RootIsFile(_)
        ));
        assert!(matches!(
            plan_create(&dir.path().join("x"), "CON", None, None, 1).expect_err("name"),
            RegistryError::UnsafeName(_)
        ));
    }

    #[test]
    fn spec_f103_link_creates_nothing_on_disk() {
        let dir = crate::test_tempdir();
        let root = dir.path().join("UserFolder");
        std::fs::create_dir_all(root.join("mods")).expect("user mods");
        let before = names(&root);

        let row = link_instance(&db(dir.path()), &root, "User", 5).expect("link");

        assert_eq!(
            names(&root),
            before,
            "nothing written into the user's folder"
        );
        assert!(
            !metadata::mirror_path(&root).exists(),
            "no mirror for a linked folder"
        );
        assert_eq!(row.source, InstanceSource::Manual);
        assert_eq!(row.minecraft_version, None, "detection is F-092/093");
        assert_eq!(row.loader, None);

        let listed = instances::list(&db(dir.path())).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "User");
    }

    #[test]
    fn spec_f103_link_refuses_the_same_folder_twice() {
        let dir = crate::test_tempdir();
        let root = dir.path().join("UserFolder");
        std::fs::create_dir_all(&root).expect("mkdir");
        let d = db(dir.path());
        link_instance(&d, &root, "One", 1).expect("first");
        let err = link_instance(&d, &root, "Two", 2).expect_err("dup");
        assert!(
            matches!(err, RegistryError::Row(InstanceRowError::DuplicatePath(_))),
            "got {err:?}"
        );
        assert_eq!(instances::list(&d).expect("list").len(), 1);
    }
}
