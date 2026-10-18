//! Async-free download queue core: retry policy (pure, unit-tested).
//!
//! The network flow itself lives here too once written; this bite is the
//! retry policy only (F-039), so the numbers are pinned before any socket.

pub mod batch;
pub mod client;
pub mod retry;

#[cfg(test)]
mod batch_tests;

pub use batch::{execute_batch, BatchError, BatchItem, ItemProgress, JournalOp};
pub use client::{
    DownloadOutcome, DownloadPhase, DownloadProgress, DownloadRequest, DownloadStatus, Downloader,
};
