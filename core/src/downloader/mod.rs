//! Async-free download queue core: retry policy (pure, unit-tested).
//!
//! The network flow itself lives here too once written; this bite is the
//! retry policy only (F-039), so the numbers are pinned before any socket.

pub mod client;
pub mod retry;
