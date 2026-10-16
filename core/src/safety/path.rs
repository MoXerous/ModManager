//! Archive member-name validation (`F-041`, `F-047`).
//!
//! Mirrors `safe_archive_member_name` in `safe_archive.py`: normalize (NFC,
//! backslash to slash) first, then reject in a fixed order. Returns the
//! normalized forward-slash form on success.

use unicode_normalization::UnicodeNormalization;

/// Limits that apply to a single member name (`F-043` values used here).
pub const MAX_NAME_LENGTH: usize = 1024;
pub const MAX_PATH_DEPTH: usize = 64;

/// Windows reserved device names, compared case-insensitively (`F-047`).
const RESERVED_NAMES: &[&str] = &[
    "aux", "con", "nul", "prn", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Why a member name was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    Empty,
    ControlChars,
    TooLong,
    Absolute,
    UnsafePath,
    TooDeep,
    AmbiguousComponent,
    Colon,
    ReservedName,
}

/// Validate `name` and return its normalized form, or the rejection reason.
///
/// Rejection order follows `safe_archive_member_name`: empty, control chars,
/// length, absolute, unsafe components, depth, ambiguous, colon, reserved.
pub fn safe_member_name(name: &str) -> Result<String, NameError> {
    let normalized: String = name.replace('\\', "/").nfc().collect();
    if normalized.is_empty() {
        return Err(NameError::Empty);
    }
    if normalized.chars().any(|c| (c as u32) < 32) {
        return Err(NameError::ControlChars);
    }
    if normalized.len() > MAX_NAME_LENGTH {
        return Err(NameError::TooLong);
    }
    if normalized.starts_with('/')
        || (normalized.len() >= 3
            && normalized.as_bytes()[1] == b':'
            && normalized.as_bytes()[2] == b'/'
            && normalized.as_bytes()[0].is_ascii_alphabetic())
    {
        return Err(NameError::Absolute);
    }
    // Directories end with '/': validate the path without it, keep it in output.
    let check = normalized.strip_suffix('/').unwrap_or(&normalized);
    if check.is_empty() {
        return Err(NameError::UnsafePath);
    }
    let parts: Vec<&str> = check.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return Err(NameError::UnsafePath);
    }
    if parts.len() > MAX_PATH_DEPTH {
        return Err(NameError::TooDeep);
    }
    if parts.iter().any(|p| *p != p.trim() || p.ends_with('.')) {
        return Err(NameError::AmbiguousComponent);
    }
    if parts.iter().any(|p| p.contains(':')) {
        return Err(NameError::Colon);
    }
    if parts.iter().any(|p| {
        let stem = p.split('.').next().unwrap_or("");
        RESERVED_NAMES.contains(&stem.to_lowercase().as_str())
    }) {
        return Err(NameError::ReservedName);
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_f041_accepts_safe_member_name() {
        assert_eq!(
            safe_member_name("mods/example.jar"),
            Ok("mods/example.jar".to_string())
        );
        // Backslashes become forward slashes.
        assert_eq!(
            safe_member_name("mods\\example.jar"),
            Ok("mods/example.jar".to_string())
        );
        // Directory entries keep their trailing slash.
        assert_eq!(safe_member_name("mods/"), Ok("mods/".to_string()));
    }

    #[test]
    fn spec_f041_rejects_empty_and_control_names() {
        assert_eq!(safe_member_name(""), Err(NameError::Empty));
        assert_eq!(safe_member_name("a\x00b.jar"), Err(NameError::ControlChars));
        assert_eq!(safe_member_name("a\x1fb.jar"), Err(NameError::ControlChars));
    }

    #[test]
    fn spec_f041_rejects_absolute_member_path() {
        assert_eq!(safe_member_name("/absolute.txt"), Err(NameError::Absolute));
        assert_eq!(safe_member_name("C:/drive.txt"), Err(NameError::Absolute));
        assert_eq!(safe_member_name("C:\\drive.txt"), Err(NameError::Absolute));
        // UNC becomes //server/... after normalization, hence absolute.
        assert_eq!(
            safe_member_name("\\\\server\\share\\x"),
            Err(NameError::Absolute)
        );
    }

    #[test]
    fn spec_f041_rejects_parent_traversal() {
        assert_eq!(safe_member_name("../evil.txt"), Err(NameError::UnsafePath));
        assert_eq!(
            safe_member_name("folder/./alias.txt"),
            Err(NameError::UnsafePath)
        );
        assert_eq!(
            safe_member_name("folder//alias.txt"),
            Err(NameError::UnsafePath)
        );
    }

    #[test]
    fn spec_f041_rejects_overlong_and_deep_paths() {
        assert_eq!(safe_member_name(&"a".repeat(1025)), Err(NameError::TooLong));
        let deep = (0..65)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/")
            + "/x.jar";
        assert_eq!(safe_member_name(&deep), Err(NameError::TooDeep));
    }

    #[test]
    fn spec_f041_rejects_ambiguous_components() {
        assert_eq!(
            safe_member_name("folder/trailing. /file.txt"),
            Err(NameError::AmbiguousComponent)
        );
        assert_eq!(
            safe_member_name("folder/ lead.txt"),
            Err(NameError::AmbiguousComponent)
        );
    }

    #[test]
    fn spec_f041_rejects_colon_components() {
        // Drive-relative (C:x) has no slash after the colon: caught by the colon rule.
        assert_eq!(safe_member_name("C:x"), Err(NameError::Colon));
        assert_eq!(safe_member_name("a/b:c.jar"), Err(NameError::Colon));
    }

    #[test]
    fn spec_f047_rejects_windows_reserved_names() {
        assert_eq!(
            safe_member_name("folder/CON.txt"),
            Err(NameError::ReservedName)
        );
        assert_eq!(safe_member_name("nul"), Err(NameError::ReservedName));
        assert_eq!(safe_member_name("com1"), Err(NameError::ReservedName));
        // Non-reserved names with similar prefixes are fine.
        assert!(safe_member_name("console.txt").is_ok());
    }
}
