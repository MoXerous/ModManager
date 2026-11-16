//! Dated-folder helpers shared by the content-remove and backup runs (F-111,
//! F-113).
//!
//! Those two features write a dated folder next to the content they describe.
//! Two copies of the timestamp format would mean two folder orders that do not
//! sort together, so the format lives here once, moved out of
//! `instances/remove` without changing behaviour.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// UTC ISO-8601 seconds. Done locally instead of pulling a date crate for one
/// folder name: the conversion is small, total, and tested at the epoch.
pub(crate) fn utc_timestamp(now: SystemTime) -> String {
    let epoch = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = epoch.div_euclid(86_400);
    let seconds = epoch.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

/// Folder-safe form of an ISO timestamp, capped like the legacy app.
pub(crate) fn backup_stamp(created_at: &str) -> String {
    let cleaned: String = created_at
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        "profile_content_remove".to_string()
    } else {
        trimmed.chars().take(64).collect()
    }
}

/// `stem`, then `stem-1`, `stem-2`, ... preserving the extension. Used for
/// both the dated folder and each file, so a second run in the same second
/// never lands on an existing destination.
pub(crate) fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().map(PathBuf::from).unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_string);
    for i in 1..10_000 {
        let name = match &extension {
            Some(ext) => format!("{stem}-{i}.{ext}"),
            None => format!("{stem}-{i}"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    // Exhausting ten thousand names means something is very wrong; returning
    // the original makes execution refuse an existing destination, never
    // overwrite one.
    path
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to a proleptic
/// Gregorian date. Integer-only, so no locale or time zone can leak in.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month as u32, day as u32)
}
