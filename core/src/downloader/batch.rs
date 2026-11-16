//! F-063: run a resolved install plan as one journalled batch.
//!
//! This is the piece the UI had no way to reach: the downloader and the
//! journal each existed, but nothing drove both. `execute_install` (Tauri)
//! and `mm-cli install` both call this, so what the user sees on screen and
//! what happens on the command line cannot drift apart.
//!
//! Progress is reported through a plain callback rather than a channel type,
//! so this module has no idea a UI exists. Throttling is the caller's job —
//! `download()` fires per chunk and says so in its own documentation.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::downloader::{
    DownloadPhase, DownloadProgress, DownloadRequest, DownloadStatus, Downloader,
};
use crate::journal::{
    ItemState, JournalError, JournalOp, OperationItem, OperationRecord, OperationState,
};

/// What the caller learns about one item's progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemProgress {
    Started {
        done: u64,
        total: Option<u64>,
    },
    Verifying,
    /// Terminal. `path` is where the file landed.
    Landed {
        path: PathBuf,
    },
    Skipped {
        path: PathBuf,
    },
    Failed {
        message: String,
    },
    Cancelled,
}

/// F-020: the batch's own totals, beside the per-item events.
///
/// One file's `done`/`total` cannot produce the overall bar the old app
/// showed, so the batch reports what the whole batch has: bytes settled across
/// every item, against the sum of every item's size. `total` is `None` when
/// any size is unknown — a bar that guesses would be a lie.
///
/// `terminal` is the batch's explicit final answer (F-020): it fires after
/// the loop AND on every error or cancel path. It cannot be inferred from
/// `done == total` — an unknown total never satisfies that, so the final
/// event would be swallowable by the throttle and the bar would hang — and a
/// batch that stops early must still tell the UI it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchProgress {
    pub done: u64,
    pub total: Option<u64>,
    pub terminal: bool,
}

/// One file to fetch, already resolved by `resolve_and_plan`.
#[derive(Debug, Clone)]
pub struct BatchItem {
    pub item_id: String,
    pub project_id: String,
    pub title: String,
    pub filename: String,
    pub url: String,
    pub size: Option<u64>,
    pub sha1_hex: Option<String>,
    pub sha512_hex: Option<String>,
    /// The provider's declared Minecraft versions for this version. Carried to
    /// the lock record at the end of the install, because that is the only
    /// moment the catalogue's own answer is known (F-097): a later scan has no
    /// network, and the jar may declare a range or nothing.
    pub game_versions: Vec<String>,
    /// Passed straight through to [`DownloadRequest`]. Production items leave
    /// this false, which is what makes the downloader https-only; a test
    /// serving wiremock on loopback sets it, exactly as the downloader's own
    /// tests do.
    pub allow_insecure_loopback: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    #[error("journal: {0}")]
    Journal(#[from] JournalError),
    #[error("download {item_id}: {message}")]
    Download { item_id: String, message: String },
    #[error("cancelled")]
    Cancelled,
}

/// The journal writes the batch needs are [`JournalOp`], which lives with the
/// journal itself (F-141): this module and the pack executor are two callers
/// of one record format, and a re-export keeps every existing caller
/// compiling without a second definition.
///
/// Runs every item, journalling the batch as it goes.
///
/// `write` is called once per state change and must not be held open between
/// calls; that is what keeps the journal available to the rest of the app
/// while a download runs.
///
/// `on_progress` is called for every item event. Returning `Err(Cancelled)`
/// stops the batch cleanly: the item in flight keeps its `.part` so a retry
/// resumes rather than restarting, and every item already landed stays landed.
///
/// `on_batch` (F-020) is called with the batch's own totals: once before the
/// first item, on every download chunk, and after every item settles. It is a
/// separate callback because the aggregate is not about one file, and the
/// per-item contract above stays exactly as it was.
pub async fn execute_batch(
    downloader: &Downloader,
    write: &mut (dyn FnMut(JournalOp) -> Result<OperationRecord, JournalError> + Send),
    target: &Path,
    items: &[BatchItem],
    cancel: Arc<AtomicBool>,
    on_progress: &mut (dyn FnMut(&str, ItemProgress) + Send),
    on_batch: &mut (dyn FnMut(BatchProgress) + Send),
) -> Result<Vec<PathBuf>, BatchError> {
    let journal_items: Vec<OperationItem> = items
        .iter()
        .map(|i| OperationItem {
            item_id: i.item_id.clone(),
            project_id: i.project_id.clone(),
            title: i.title.clone(),
            filename: i.filename.clone(),
            state: ItemState::Queued,
            partial_path: String::new(),
            final_path: String::new(),
            message: String::new(),
        })
        .collect();

    // The journal is written before the first byte moves: a crash with no
    // record leaves files on disk that nothing claims, and that is the
    // recoverable direction (AGENTS.md §5.4).
    let record = write(JournalOp::Start {
        target: target.to_string_lossy().into_owned(),
        items: journal_items,
    })?;
    let _ = &record;
    write(JournalOp::Operation(OperationState::Running))?;

    // F-020: the batch reports its own totals beside the per-item events.
    // `Option<u64>` sums to `None` when one size is unknown.
    let batch_total: Option<u64> = items.iter().map(|i| i.size).sum();
    let mut settled_bytes: u64 = 0;
    // F-020: the reported `done` is a HIGH-WATER mark. A resume that is
    // restarted (the server ignores `Range`, or answers 416) resets one
    // file's `done` to zero while the settled bytes stay put, so a plain
    // `settled_bytes + p.done` would move the bar BACKWARDS — and a bar that
    // rewinds is a lie. Progress may stall while the restart re-fetches; it
    // may never go down.
    let mut high_water: u64 = 0;
    on_batch(BatchProgress {
        done: 0,
        total: batch_total,
        terminal: false,
    });

    let mut landed: Vec<PathBuf> = Vec::with_capacity(items.len());
    for item in items {
        if cancel.load(Ordering::Relaxed) {
            // F-036: the cancellation stands whatever the journal says. A
            // write failure here used to be either swallowed or propagated
            // in place of the cancel — both wrong. It is recorded, and the
            // cancel is still the answer the caller gets.
            if let Err(e) = write(JournalOp::Operation(OperationState::Cancelled)) {
                tracing::warn!(
                    error = %e,
                    "could not record the cancelled operation state; the cancellation stands"
                );
            }
            on_batch(BatchProgress {
                done: high_water,
                total: batch_total,
                terminal: true,
            });
            return Err(BatchError::Cancelled);
        }

        let dest = target.join(&item.filename);
        let req = DownloadRequest {
            url: item.url.clone(),
            dest: dest.clone(),
            expected_size: item.size,
            sha1_hex: item.sha1_hex.clone(),
            sha512_hex: item.sha512_hex.clone(),
            allow_insecure_loopback: item.allow_insecure_loopback,
        };

        // The item must walk the same states the phase does. Jumping straight
        // to Completed is rejected by the state machine, and it would be worse
        // if it were not: a record that only ever says "Completed" cannot say
        // where a crash caught it, which is the whole reason it is written.
        let mut last_phase: Option<ItemState> = None;
        let mut journal_err: Option<BatchError> = None;
        // The in-flight item's latest byte count, for the batch totals.
        let mut current_done: u64 = 0;
        let mut emit = |p: DownloadProgress| {
            // F-036: a flipped flag must END the download, not park it. The
            // downloader polls this callback from its pause loop too, and
            // only a `Break` turns the flip into the cancelled status that
            // keeps the `.part` for a retry to resume.
            if cancel.load(Ordering::Relaxed) {
                return ControlFlow::Break(());
            }
            if journal_err.is_some() {
                return ControlFlow::Continue(());
            }
            let next = match p.phase {
                DownloadPhase::Downloading => ItemState::Downloading,
                DownloadPhase::Verifying => ItemState::Verifying,
                DownloadPhase::Committing => ItemState::Committing,
            };
            // F-020: every chunk moves the batch bar, even when the per-item
            // event is a suppressed repeat. The high-water mark keeps the
            // bar honest across a restarted resume (see the comment above).
            current_done = p.done;
            high_water = high_water.max(settled_bytes + p.done);
            on_batch(BatchProgress {
                done: high_water,
                total: batch_total,
                terminal: false,
            });
            if last_phase == Some(next) {
                return ControlFlow::Continue(());
            }
            last_phase = Some(next);
            if let Err(e) = write(JournalOp::Item {
                item_id: item.item_id.clone(),
                state: next,
            }) {
                journal_err = Some(BatchError::Journal(e));
                return ControlFlow::Continue(());
            }
            on_progress(
                &item.item_id,
                match p.phase {
                    DownloadPhase::Downloading => ItemProgress::Started {
                        done: p.done,
                        total: p.total,
                    },
                    _ => ItemProgress::Verifying,
                },
            );
            ControlFlow::Continue(())
        };
        let is_cancelled = || cancel.load(Ordering::Relaxed);

        let result = downloader.download(&req, &mut emit, &is_cancelled).await;
        if let Some(e) = journal_err {
            // F-020: the batch stops here, so it says so — the terminal event
            // fires on the error path too, not only after a clean loop.
            on_batch(BatchProgress {
                done: high_water,
                total: batch_total,
                terminal: true,
            });
            return Err(e);
        }
        match result {
            Ok(outcome) => {
                let item_state = match outcome.status {
                    DownloadStatus::Cancelled => {
                        // F-036: the operation itself must read as cancelled,
                        // not stay `running` — a record stuck at running is
                        // offered as unfinished work after a restart, and a
                        // cancelled batch has no unfinished work to offer.
                        // F-036 (journal): a write failure is recorded, never
                        // swallowed and never allowed to replace the
                        // cancellation as the caller's answer.
                        if let Err(e) = write(JournalOp::Item {
                            item_id: item.item_id.clone(),
                            state: ItemState::Cancelled,
                        }) {
                            tracing::warn!(
                                error = %e,
                                item_id = %item.item_id,
                                "could not record the cancelled item; the cancellation stands"
                            );
                        }
                        if let Err(e) = write(JournalOp::Operation(OperationState::Cancelled)) {
                            tracing::warn!(
                                error = %e,
                                "could not record the cancelled operation state; the cancellation stands"
                            );
                        }
                        on_progress(&item.item_id, ItemProgress::Cancelled);
                        on_batch(BatchProgress {
                            done: high_water,
                            total: batch_total,
                            terminal: true,
                        });
                        return Err(BatchError::Cancelled);
                    }
                    DownloadStatus::Skipped => ItemState::Skipped,
                    DownloadStatus::Success => ItemState::Completed,
                };
                let landed_path = outcome.path;
                let _ = write(JournalOp::Item {
                    item_id: item.item_id.clone(),
                    state: item_state,
                })?;
                let reported = if item_state == ItemState::Skipped {
                    ItemProgress::Skipped {
                        path: landed_path.clone(),
                    }
                } else {
                    ItemProgress::Landed {
                        path: landed_path.clone(),
                    }
                };
                if item_state == ItemState::Completed {
                    landed.push(landed_path);
                }
                on_progress(&item.item_id, reported);
                // F-020: the settled item's bytes fold into the batch totals.
                // A skipped file was already on disk, so its size counts as
                // done; a file with no declared size contributes what it
                // actually moved.
                settled_bytes += match item_state {
                    ItemState::Skipped | ItemState::Completed => item.size.unwrap_or(current_done),
                    _ => current_done,
                };
                high_water = high_water.max(settled_bytes);
                on_batch(BatchProgress {
                    done: high_water,
                    total: batch_total,
                    terminal: false,
                });
            }
            Err(e) => {
                // One bad file fails its own item, not the whole batch: the
                // files already landed stay landed and stay recorded.
                let _ = write(JournalOp::Item {
                    item_id: item.item_id.clone(),
                    state: ItemState::Failed,
                })?;
                on_progress(
                    &item.item_id,
                    ItemProgress::Failed {
                        message: e.to_string(),
                    },
                );
                // F-020: whatever the failed file moved still counts.
                settled_bytes += current_done;
                high_water = high_water.max(settled_bytes);
                on_batch(BatchProgress {
                    done: high_water,
                    total: batch_total,
                    terminal: false,
                });
                // F-020: the batch stops on the failure, so the terminal
                // event fires here too — an error is exactly when the bar
                // would otherwise freeze.
                on_batch(BatchProgress {
                    done: high_water,
                    total: batch_total,
                    terminal: true,
                });
                return Err(BatchError::Download {
                    item_id: item.item_id.clone(),
                    message: e.to_string(),
                });
            }
        }
    }

    let _ = write(JournalOp::Operation(OperationState::Completed))?;
    // F-020: the loop is over and the batch succeeded — the terminal event
    // is explicit, because `done == total` never holds when the total is
    // unknown and a swallowed final event would leave the bar short.
    high_water = high_water.max(settled_bytes);
    on_batch(BatchProgress {
        done: high_water,
        total: batch_total,
        terminal: true,
    });
    Ok(landed)
}
