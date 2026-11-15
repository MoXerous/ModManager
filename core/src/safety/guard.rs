//! Link and reparse-point guards for paths we write through (`F-038`).
//!
//! A check made *before* opening cannot close the window: the path can be
//! replaced by a link between the check and the `open` call, and an open that
//! follows it writes outside the directory the caller intended. The pattern
//! is therefore **open, then validate the handle, then write**
//! (`docs/spec/behaviors/downloads.md` §8):
//!
//! 1. [`is_link_or_reparse`] once more before opening — defence in depth;
//! 2. [`do_not_follow`] so the final component is never followed;
//! 3. [`handle_is_link_or_reparse`] on the opened handle, which is the truth
//!    about what the caller holds — even when the path is swapped afterwards.
//!
//! [`open_no_follow`] is the three steps in one place: it is what the
//! downloader and the atomic commit use, so neither can spell the order
//! differently.
//!
//! Two kinds of link matter. A POSIX symlink, and on Windows every *reparse
//! point*: a symbolic link **or an NTFS junction**.
//! [`std::fs::FileType::is_symlink`] misses junctions, so the reparse
//! attribute is checked explicitly.
//!
//! The *last* component is not the whole path: a directory on the way to it
//! can be a junction too, and then every file written "inside" it lands
//! somewhere else. [`link_inside`] and [`link_at_or_inside`] walk those
//! components without following them; archive extraction, the pack executor
//! and the downloader all use the same walk.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Windows `FILE_ATTRIBUTE_REPARSE_POINT`.
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
/// Windows `FILE_FLAG_OPEN_REPARSE_POINT`: open the reparse point itself
/// instead of resolving it.
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
/// Linux `O_NOFOLLOW` (`include/uapi/asm-generic/fcntl.h`: `#define O_NOFOLLOW
/// 0400000`, octal).
#[cfg(any(target_os = "linux", target_os = "android"))]
const O_NOFOLLOW: i32 = 0o400_000;
/// macOS/iOS `O_NOFOLLOW`. Source: XNU `bsd/sys/fcntl.h`
/// (`#define O_NOFOLLOW 0x00000100`) and `open(2)` on macOS.
///
/// `0x200` is **`O_CREAT`** on macOS and the BSDs, not "do not follow": using
/// it made the open create a brand-new file *and* follow the planted link, so
/// the handle looked like an ordinary file and the guard passed.
#[cfg(any(target_os = "macos", target_os = "ios"))]
const O_NOFOLLOW: i32 = 0x100;

/// Why a guarded open refused.
#[derive(Debug, Error)]
pub enum GuardError {
    /// This target has no no-follow flag this code can vouch for. Refusing is
    /// the only safe answer: opening anyway would follow a link silently.
    #[error("this platform has no verified no-follow open flag; refusing to open unguarded")]
    UnsupportedPlatform,
    #[error("refusing to open a link or reparse point at {}", .0.display())]
    LinkOrReparse(PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Is the path *itself* a symlink or reparse point? Never follows the link.
///
/// Path-level only: a caller that is about to open the path must still
/// validate the handle afterwards (that is the point of this module).
pub fn is_link_or_reparse(path: &Path) -> bool {
    match path.symlink_metadata() {
        Ok(meta) => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                meta.file_type().is_symlink()
                    || (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
            }
            #[cfg(not(windows))]
            {
                meta.file_type().is_symlink()
            }
        }
        Err(_) => false,
    }
}

/// Did the opened handle land on something we must not write through?
///
/// The handle's own metadata is used, never the path: a path can be swapped
/// after the open, the handle cannot. Metadata errors fail closed.
///
/// * Windows: the handle was opened with
///   [`FILE_FLAG_OPEN_REPARSE_POINT`], so a `FILE_ATTRIBUTE_REPARSE_POINT`
///   here means the caller is holding the link, not its target.
/// * Unix: `O_NOFOLLOW` refuses a symlink at open, so a handle can never be a
///   symlink — the rule is the stronger one: a handle we write through must
///   be a regular file.
pub fn handle_is_link_or_reparse(file: &File) -> bool {
    let Ok(meta) = file.metadata() else {
        return true;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink()
            || (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
    }
    #[cfg(not(windows))]
    {
        !meta.file_type().is_file()
    }
}

/// Make `opts` refuse to follow a link in the final path component.
///
/// Windows opens the reparse point itself
/// ([`FILE_FLAG_OPEN_REPARSE_POINT`]) so the handle can be inspected; Unix
/// fails the open outright (`O_NOFOLLOW`). Either way the link is never
/// traversed.
///
/// A target this module has no verified flag for gets
/// [`GuardError::UnsupportedPlatform`] — there is deliberately no "open
/// normally" fallback, because that would follow the link.
pub fn do_not_follow(opts: &mut OpenOptions) -> Result<(), GuardError> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        Ok(())
    }
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(O_NOFOLLOW);
        Ok(())
    }
    #[cfg(not(any(
        windows,
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    {
        // FreeBSD, NetBSD, OpenBSD, Solaris, …: `O_NOFOLLOW` differs per
        // kernel and none of them is a shipped target, so guessing could
        // silently mean something else (as `0x200` did on macOS). Refuse.
        let _ = opts;
        Err(GuardError::UnsupportedPlatform)
    }
}

/// Open with no follow, then judge the *handle* — never only the path.
///
/// The link is refused with [`GuardError::LinkOrReparse`], whether the kernel
/// failed the open (Unix `O_NOFOLLOW`) or handed back the reparse point itself
/// (Windows).
pub fn open_no_follow(path: &Path, opts: &mut OpenOptions) -> Result<File, GuardError> {
    do_not_follow(opts)?;
    match opts.open(path) {
        Ok(file) => {
            if handle_is_link_or_reparse(&file) {
                // Close first: Windows refuses to unlink a file with an open
                // handle.
                drop(file);
                Err(GuardError::LinkOrReparse(path.to_path_buf()))
            } else {
                Ok(file)
            }
        }
        // Unix `O_NOFOLLOW` fails the open outright instead of handing back a
        // handle to the link; the path is what tells the two errors apart.
        Err(_) if is_link_or_reparse(path) => Err(GuardError::LinkOrReparse(path.to_path_buf())),
        Err(e) => Err(GuardError::Io(e)),
    }
}

/// The first link or reparse point between `target`'s parent and `root`,
/// judging everything *below* `root` but not `root` itself.
///
/// The caller vouches for `root` (it created it, or the user chose it); a
/// path that is not below `root` is not judged at all.
pub fn link_inside(root: &Path, target: &Path) -> Option<PathBuf> {
    walk(root, target, false)
}

/// Like [`link_inside`], but `root` itself is judged too.
///
/// Use this when `root` is the directory the bytes land in — the replaced
/// `mods/` junction the downloader must refuse.
pub fn link_at_or_inside(root: &Path, target: &Path) -> Option<PathBuf> {
    walk(root, target, true)
}

/// The first link or reparse point on the way to `target`, judging *every*
/// ancestor from `target`'s parent up to the filesystem root.
///
/// The downloader has no vouched root: it writes wherever the caller chose,
/// so every component of the path the bytes travel through must be a real
/// directory. This is the whole-path walk the old app ran before each write;
/// the same [`is_link_or_reparse`] rule judges each component.
pub fn link_in_ancestors(target: &Path) -> Option<PathBuf> {
    let mut ancestor = target.parent();
    while let Some(dir) = ancestor {
        if is_link_or_reparse(dir) {
            return Some(dir.to_path_buf());
        }
        match dir.parent() {
            Some(p) if p != dir => ancestor = Some(p),
            // Root component reached (or a degenerate parent): done.
            _ => return None,
        }
    }
    None
}

fn walk(root: &Path, target: &Path, include_root: bool) -> Option<PathBuf> {
    let mut ancestor = target.parent();
    while let Some(dir) = ancestor {
        if dir == root {
            return (include_root && is_link_or_reparse(dir)).then(|| dir.to_path_buf());
        }
        if !dir.starts_with(root) {
            // Not under the directory the caller vouched for: nothing here
            // belongs to this tree, and walking further would judge the
            // user's own filesystem (a relocated home, `/var` on macOS).
            return None;
        }
        if is_link_or_reparse(dir) {
            return Some(dir.to_path_buf());
        }
        ancestor = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_f038_handle_rule_accepts_a_regular_file() {
        let dir = crate::test_tempdir();
        let path = dir.path().join("plain.bin");
        std::fs::write(&path, b"bytes").expect("fixture");
        let file = File::open(&path).expect("open");
        assert!(!handle_is_link_or_reparse(&file));
        assert!(!is_link_or_reparse(&path));
    }

    #[test]
    fn spec_f038_missing_path_is_not_reported_as_a_link() {
        // Absence is not a link: the caller decides what a missing path means.
        let dir = crate::test_tempdir();
        assert!(!is_link_or_reparse(&dir.path().join("absent.bin")));
    }

    #[test]
    fn spec_f038_ancestor_walk_does_not_judge_paths_outside_the_root() {
        // A root the target is not under is not ours to police: walking to
        // the filesystem root would refuse every write on a machine whose
        // home or `/var` is itself a link.
        let dir = crate::test_tempdir();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).expect("root");
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("elsewhere");
        assert_eq!(link_at_or_inside(&root, &elsewhere.join("x.jar")), None);
        assert_eq!(
            link_at_or_inside(&root, &root.join("sub").join("x.jar")),
            None,
            "a path that does not exist yet has no link"
        );
    }
}
