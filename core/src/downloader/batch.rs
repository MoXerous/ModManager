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
use crate::journal::{ItemState, JournalError, OperationItem, OperationRecord, OperationState};

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

/// One journal write the batch needs.
///
/// A callback rather than a `&Journal`, and that is the whole point: a
/// `&Journal` held across a multi-minute download means the mutex behind it is
/// held for minutes too, and every other command that touches the journal —
/// including the recovery reads on the Instances page — waits for the last
/// byte. This type lets the caller open and close the lock per write.
#[derive(Debug, Clone)]
pub enum JournalOp {
    Start {
        target: String,
        items: Vec<OperationItem>,
    },
    Operation(OperationState),
    Item {
        item_id: String,
        state: ItemState,
    },
}

/// Runs every item, journalling the batch as it goes.
///
/// `write` is called once per state change and must not be held open between
/// calls; that is what keeps the journal available to the rest of the app
/// while a download runs.
///
/// `on_progress` is called for every item event. Returning `Err(Cancelled)`
/// stops the batch cleanly: the item in flight keeps its `.part` so a retry
/// resumes rather than restarting, and every item already landed stays landed.
pub async fn execute_batch(
    downloader: &Downloader,
    write: &mut dyn FnMut(JournalOp) -> Result<OperationRecord, JournalError>,
    target: &Path,
    items: &[BatchItem],
    cancel: Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(&str, ItemProgress),
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

    let mut landed: Vec<PathBuf> = Vec::with_capacity(items.len());
    for item in items {
        if cancel.load(Ordering::Relaxed) {
            let _ = write(JournalOp::Operation(OperationState::Cancelled))?;
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
        let mut emit = |p: DownloadProgress| {
            if journal_err.is_some() {
                return ControlFlow::Continue(());
            }
            let next = match p.phase {
                DownloadPhase::Downloading => ItemState::Downloading,
                DownloadPhase::Verifying => ItemState::Verifying,
                DownloadPhase::Committing => ItemState::Committing,
            };
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
            return Err(e);
        }
        match result {
            Ok(outcome) => {
                let item_state = match outcome.status {
                    DownloadStatus::Cancelled => {
                        let _ = write(JournalOp::Item {
                            item_id: item.item_id.clone(),
                            state: ItemState::Cancelled,
                        })?;
                        on_progress(&item.item_id, ItemProgress::Cancelled);
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
                return Err(BatchError::Download {
                    item_id: item.item_id.clone(),
                    message: e.to_string(),
                });
            }
        }
    }

    let _ = write(JournalOp::Operation(OperationState::Completed))?;
    Ok(landed)
}
