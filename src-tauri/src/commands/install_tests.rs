//! F-036: a cancelled install crosses IPC as `cancelled`, never as `io`.

use mm_core::downloader::BatchError;

use super::install_batch_error;
use crate::dto::ErrorCode;

#[test]
fn spec_f036_cancelled_maps_to_the_cancelled_code_not_io() {
    let e = install_batch_error(&BatchError::Cancelled);
    assert_eq!(e.code, ErrorCode::Cancelled);
    // The message is diagnostic only; the UI translates the code.
    assert!(e.message.contains("cancel"));
}

#[test]
fn spec_f036_download_and_journal_failures_stay_io() {
    let cases = [
        BatchError::Download {
            item_id: "v1".into(),
            message: "hash mismatch".into(),
        },
        BatchError::Journal(mm_core::journal::JournalError::ItemMissing("v1".into())),
    ];
    for e in &cases {
        let mapped = install_batch_error(e);
        assert_eq!(
            mapped.code,
            ErrorCode::Io,
            "a real failure must not read as a cancellation"
        );
    }
}
