//! F-168: a read-only filesystem watcher over an instance's content folders.
//!
//! The core owns the rules (debounce, suppression of the app's own writes);
//! the Tauri layer only carries the signal to the webview. This module never
//! writes to the watched folders.
//!
//! Three rules, each independently testable:
//!
//! * **Debounce** — events arriving inside the quiet window are one burst,
//!   and a burst signals once, when the window has elapsed without a new
//!   event. A burst of a dozen writes collapses to one signal.
//! * **Suppression** — a burst whose every path sits under a root the app
//!   itself wrote to recently (an install, a toggle, a restore) is dropped:
//!   the app's own writes must not tell the user their content is stale.
//! * **Read-only** — the watcher only reads directory events. It holds no
//!   file handle it writes through and never touches the watched tree.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher as _};
use thiserror::Error;

/// Events arriving inside this window are one burst: one signal for the UI.
/// The brief asks for ~200 ms between the events on disk and the frontend.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(200);

/// A write the app itself made suppresses events under the same root for
/// this long. Longer than an install's trailing cleanup, short enough that a
/// real external change right after an install is not swallowed for long.
pub const DEFAULT_SUPPRESSION: Duration = Duration::from_secs(10);

/// How often the loop wakes to check the burst window and the stop flag.
const POLL: Duration = Duration::from_millis(25);

/// The registry of the app's own recent writes, keyed by the root it wrote
/// into. Shared between the write commands (which note their writes) and the
/// watcher (which consults it before signalling).
pub type OwnWrites = Arc<Mutex<HashMap<PathBuf, Instant>>>;

pub fn new_own_writes() -> OwnWrites {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Records that the app is writing under `root` as of now. Write commands
/// call this when their batch starts and when it settles.
pub fn note_own_write(writes: &OwnWrites, root: &Path) {
    if let Ok(mut map) = writes.lock().map_err(PoisonError::into_inner) {
        map.insert(root.to_path_buf(), Instant::now());
    }
}

/// The suppression rule: an event path is suppressed when any write root is
/// an ancestor of it (or the same path) and that write happened inside the
/// window. Pure, so the rule is testable without an OS watcher.
pub fn suppressed(
    path: &Path,
    writes: &[(PathBuf, Instant)],
    now: Instant,
    window: Duration,
) -> bool {
    writes
        .iter()
        .any(|(root, at)| path.starts_with(root) && now.duration_since(*at) <= window)
}

/// F-168: an event path is relevant when it sits under one of the content
/// folders. Pure, so the rule is testable without an OS watcher. Empty
/// prefixes admit everything, so a watcher without the instance shape keeps
/// today's behaviour.
pub fn relevant(path: &Path, prefixes: &[PathBuf]) -> bool {
    prefixes.is_empty() || prefixes.iter().any(|prefix| path.starts_with(prefix))
}

/// The debounce decision for one watched folder set. Admit every event path;
/// the burst is "due" once the quiet window has elapsed since the last one.
#[derive(Debug)]
pub struct BurstWindow {
    window: Duration,
    last: Option<Instant>,
    paths: Vec<PathBuf>,
}

impl BurstWindow {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            last: None,
            paths: Vec::new(),
        }
    }

    pub fn admit(&mut self, at: Instant, path: PathBuf) {
        self.last = Some(at);
        self.paths.push(path);
    }

    pub fn due(&self, now: Instant) -> bool {
        match self.last {
            Some(last) => now.duration_since(last) >= self.window,
            None => false,
        }
    }

    /// Closes the burst and returns every path it saw.
    pub fn take(&mut self) -> Vec<PathBuf> {
        self.last = None;
        std::mem::take(&mut self.paths)
    }
}

/// The durations and the relevance rule a watcher runs with; tests shrink
/// them.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    pub debounce: Duration,
    pub suppression: Duration,
    /// F-168: an event path outside every prefix here is dropped before the
    /// burst window. Empty admits everything — the rule only binds when the
    /// watched set is wider than the content folders (a fresh instance whose
    /// root is watched until `mods/` exists).
    pub relevant_under: Vec<PathBuf>,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce: DEFAULT_DEBOUNCE,
            suppression: DEFAULT_SUPPRESSION,
            relevant_under: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("the filesystem watcher failed: {0}")]
    Watch(#[from] notify::Error),
}

/// Only what changes content counts: create, modify, remove. Access events
/// (a reader opening a jar) say nothing about staleness.
fn is_content(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

/// A running watcher. `stop` tears the OS watcher and the loop down and
/// joins the thread; after it returns, the signal channel is closed.
#[derive(Debug)]
pub struct WatcherHandle {
    stop: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl WatcherHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self
            .join
            .lock()
            .map_err(PoisonError::into_inner)
            .ok()
            .and_then(|mut slot| slot.take())
        {
            let _ = join.join();
        }
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Watches `paths` recursively and sends one `()` per coalesced burst that
/// is not suppressed. The watcher is read-only: it only registers interest
/// and reads events.
pub fn watch(
    paths: Vec<PathBuf>,
    config: WatchConfig,
    writes: OwnWrites,
    signal: Sender<()>,
) -> Result<WatcherHandle, WatcherError> {
    let (tx_events, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(tx_events)?;
    for path in &paths {
        watcher.watch(path, RecursiveMode::Recursive)?;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = stop.clone();
    let join = std::thread::Builder::new()
        .name("content-watcher".to_string())
        .spawn(move || {
            // The watcher must live as long as the loop: dropping it
            // unregisters the interest, which is exactly the teardown.
            let _watcher = watcher;
            let mut burst = BurstWindow::new(config.debounce);
            loop {
                if stop_for_thread.load(Ordering::Relaxed) {
                    break;
                }
                match rx.recv_timeout(POLL) {
                    Ok(Ok(event)) => {
                        if is_content(&event.kind) {
                            let at = Instant::now();
                            for path in event.paths {
                                // F-168: a path outside the content folders
                                // (logs/, config/) says nothing about
                                // staleness, even when the whole root is
                                // watched.
                                if relevant(&path, &config.relevant_under) {
                                    burst.admit(at, path);
                                }
                            }
                        }
                    }
                    // A dropped read is advisory: the next event re-triggers.
                    // The UI's staleness hint survives a missed event.
                    Ok(Err(_)) => {}
                    Err(RecvTimeoutError::Timeout) => {
                        let now = Instant::now();
                        if !burst.due(now) {
                            continue;
                        }
                        let paths = burst.take();
                        let snapshot: Vec<(PathBuf, Instant)> = writes
                            .lock()
                            .map(|map| map.iter().map(|(k, v)| (k.clone(), *v)).collect())
                            .unwrap_or_default();
                        // One unsuppressed path is enough: the content moved.
                        if paths
                            .iter()
                            .any(|p| !suppressed(p, &snapshot, now, config.suppression))
                        {
                            let _ = signal.send(());
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .map_err(notify::Error::io)?;
    Ok(WatcherHandle {
        stop,
        join: Mutex::new(Some(join)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    fn config() -> WatchConfig {
        WatchConfig {
            debounce: Duration::from_millis(150),
            suppression: Duration::from_secs(5),
            relevant_under: Vec::new(),
        }
    }

    #[test]
    fn spec_f168_a_burst_inside_the_quiet_window_is_one_signal() {
        let mut burst = BurstWindow::new(Duration::from_millis(200));
        let now = Instant::now();
        for i in 0..5 {
            burst.admit(
                now + Duration::from_millis(i * 20),
                PathBuf::from(format!("f{i}.jar")),
            );
        }
        assert!(!burst.due(now), "an open burst is not yet due");
        assert!(
            !burst.due(now + Duration::from_millis(199)),
            "the quiet window has not elapsed"
        );
        assert!(burst.due(now + Duration::from_millis(280)));
        assert_eq!(burst.take().len(), 5, "the whole burst coalesces");
        assert!(
            !burst.due(now + Duration::from_millis(300)),
            "a drained burst is closed"
        );
    }

    #[test]
    fn spec_f168_an_event_under_a_recent_app_write_is_suppressed() {
        let writes = new_own_writes();
        let root = PathBuf::from("/instance");
        note_own_write(&writes, &root);
        let snapshot: Vec<(PathBuf, Instant)> = writes
            .lock()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default();
        let now = Instant::now();
        assert!(suppressed(
            &root.join("mods").join("x.jar"),
            &snapshot,
            now,
            DEFAULT_SUPPRESSION
        ));
    }

    #[test]
    fn spec_f168_an_event_after_the_suppression_window_passes() {
        let root = PathBuf::from("/instance");
        let writes = vec![(root.clone(), Instant::now() - DEFAULT_SUPPRESSION * 2)];
        assert!(!suppressed(
            &root.join("mods").join("x.jar"),
            &writes,
            Instant::now(),
            DEFAULT_SUPPRESSION
        ));
    }

    #[test]
    fn spec_f168_an_event_outside_the_written_root_is_not_suppressed() {
        let written = PathBuf::from("/other-instance");
        let writes = vec![(written, Instant::now())];
        assert!(!suppressed(
            &PathBuf::from("/instance").join("mods").join("x.jar"),
            &writes,
            Instant::now(),
            DEFAULT_SUPPRESSION
        ));
    }

    /// F-168: the relevance rule is pure, so the folder logic is pinned
    /// without an OS watcher.
    #[test]
    fn spec_f168_an_event_outside_the_content_folders_is_not_relevant() {
        let root = PathBuf::from("/instance");
        let prefixes = vec![root.join("mods"), root.join("disabled_mods")];
        assert!(relevant(&root.join("mods").join("a.jar"), &prefixes));
        assert!(relevant(
            &root.join("disabled_mods").join("b.jar"),
            &prefixes
        ));
        assert!(
            !relevant(&root.join("logs").join("app.log"), &prefixes),
            "a write to logs/ says nothing about staleness"
        );
        assert!(
            relevant(&root.join("logs").join("app.log"), &[]),
            "empty prefixes admit everything"
        );
    }

    /// The fresh-instance shape: `mods/` does not exist, so the instance root
    /// itself is watched. A write to `logs/` must stay quiet; the first
    /// content folder event must still signal.
    #[test]
    fn spec_f168_a_write_outside_the_content_folders_does_not_signal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = dir.path().join("logs");
        std::fs::create_dir(&logs).expect("logs dir");
        let cfg = WatchConfig {
            debounce: Duration::from_millis(300),
            suppression: Duration::from_secs(5),
            relevant_under: vec![dir.path().join("mods"), dir.path().join("disabled_mods")],
        };
        let (tx, rx) = mpsc::channel();
        let handle =
            watch(vec![dir.path().to_path_buf()], cfg, new_own_writes(), tx).expect("watch");
        std::thread::sleep(Duration::from_millis(100));
        std::fs::write(logs.join("app.log"), b"log").expect("write a log");
        let quiet = rx.recv_timeout(Duration::from_millis(1500));
        assert!(
            matches!(quiet, Err(mpsc::RecvTimeoutError::Timeout)),
            "a write to logs/ must not signal staleness, got {quiet:?}"
        );
        // The mirror half: once the content folders exist, their events are
        // exactly the ones that signal.
        std::fs::create_dir(dir.path().join("mods")).expect("mods dir");
        let signal = rx.recv_timeout(Duration::from_secs(5));
        handle.stop();
        assert!(
            signal.is_ok(),
            "an event under mods/ is staleness, got {signal:?}"
        );
    }

    /// The real watcher, against a real folder: a file the user (or any other
    /// process) drops into `mods` makes the content stale.
    #[test]
    fn spec_f168_a_file_create_in_the_watched_folder_signals_staleness() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tx, rx) = mpsc::channel();
        let handle = watch(
            vec![dir.path().to_path_buf()],
            config(),
            new_own_writes(),
            tx,
        )
        .expect("watch");
        std::thread::sleep(Duration::from_millis(100));
        std::fs::write(dir.path().join("new.jar"), b"jar").expect("write a file");
        let signal = rx.recv_timeout(Duration::from_secs(5));
        handle.stop();
        assert!(signal.is_ok(), "a create event must signal staleness");
    }

    /// A fast scroll of the finder (many files at once) is one signal, not a
    /// storm: the debounce is the whole point.
    #[test]
    fn spec_f168_a_burst_of_writes_does_not_storm_the_ui() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tx, rx) = mpsc::channel();
        let cfg = WatchConfig {
            debounce: Duration::from_millis(300),
            suppression: Duration::from_secs(5),
            relevant_under: Vec::new(),
        };
        let handle =
            watch(vec![dir.path().to_path_buf()], cfg, new_own_writes(), tx).expect("watch");
        std::thread::sleep(Duration::from_millis(100));
        for i in 0..12 {
            std::fs::write(dir.path().join(format!("burst{i}.jar")), b"jar").expect("write");
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut signals = 0;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            match rx.recv_timeout(deadline - Instant::now()) {
                Ok(()) => signals += 1,
                Err(_) => break,
            }
        }
        handle.stop();
        assert!(signals >= 1, "the burst must be signalled at all");
        assert!(
            signals <= 3,
            "12 writes in 120ms must coalesce, got {signals} signals"
        );
    }

    /// The app's own writes (an install, a toggle) must not mark the content
    /// stale: the suppression window swallows them.
    #[test]
    fn spec_f168_the_apps_own_writes_never_signal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tx, rx) = mpsc::channel();
        let writes = new_own_writes();
        note_own_write(&writes, dir.path());
        let handle = watch(vec![dir.path().to_path_buf()], config(), writes, tx).expect("watch");
        std::thread::sleep(Duration::from_millis(100));
        std::fs::write(dir.path().join("installed.jar"), b"jar").expect("write a file");
        let quiet = rx.recv_timeout(Duration::from_millis(1500));
        handle.stop();
        assert!(
            matches!(quiet, Err(mpsc::RecvTimeoutError::Timeout)),
            "the app's own write must not signal staleness, got {quiet:?}"
        );
    }

    /// Teardown is clean: after `stop`, the signal channel closes and later
    /// writes stay silent.
    #[test]
    fn spec_f168_stopping_the_watcher_closes_the_signal_channel() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tx, rx) = mpsc::channel();
        let handle = watch(
            vec![dir.path().to_path_buf()],
            config(),
            new_own_writes(),
            tx,
        )
        .expect("watch");
        std::thread::sleep(Duration::from_millis(100));
        handle.stop();
        let closed = rx.recv_timeout(Duration::from_secs(2));
        assert!(
            matches!(closed, Err(mpsc::RecvTimeoutError::Disconnected)),
            "a stopped watcher closes its channel, got {closed:?}"
        );
        std::fs::write(dir.path().join("late.jar"), b"jar").expect("write a file");
        let late = rx.recv_timeout(Duration::from_millis(500));
        assert!(
            matches!(late, Err(mpsc::RecvTimeoutError::Disconnected)),
            "a stopped watcher stays stopped, got {late:?}"
        );
    }
}
