//! `profile.json` and `installed_mods.json`: validate, count, never adopt.
//!
//! Both files are optional. A classic profile has neither, so "missing" is a
//! fact, not a warning. A present file that does not parse is also not fatal:
//! the scan keeps the profile row and reports a warning, because one corrupt
//! file must not hide the rest of the old home.

use std::path::Path;

use serde_json::Value;

use super::MAX_JSON_BYTES;

pub const PROFILE_JSON: &str = "profile.json";
pub const INSTALLED_MODS_JSON: &str = "installed_mods.json";

/// Metadata written by `metadata_from_profile`.
pub const PROFILE_SCHEMA_V1: &str = "modrinth_bulk_downloader.profile_metadata.v1";
/// The older, path-free profile document.
pub const PROFILE_SCHEMA_V2: &str = "modrinth_bulk_downloader.profile.v2";
/// Managed-install manifest.
pub const INSTALLED_MODS_SCHEMA_V1: &str = "modrinth_bulk_downloader.installed_mods.v1";

/// What a `profile.json` is: present, which schema, and whether this reader
/// accepts it. `detail` is a stable reason (`missing`, `invalid_json`, .).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonFact {
    pub present: bool,
    pub schema: String,
    pub ok: bool,
    pub detail: String,
}

impl JsonFact {
    pub fn missing() -> Self {
        Self {
            present: false,
            schema: String::new(),
            ok: false,
            detail: "missing".to_string(),
        }
    }

    fn present(schema: String) -> Self {
        Self {
            present: true,
            schema,
            ok: false,
            detail: String::new(),
        }
    }

    fn failed(mut self, detail: &str) -> Self {
        self.ok = false;
        self.detail = detail.to_string();
        self
    }
}

/// The managed-install manifest, reduced to what a preview needs: does it
/// parse, and how many records would an import consider real?
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledModsFact {
    pub present: bool,
    pub schema: String,
    pub ok: bool,
    pub managed: usize,
    pub skipped: usize,
    pub detail: String,
}

impl InstalledModsFact {
    pub fn missing() -> Self {
        Self {
            present: false,
            schema: String::new(),
            ok: false,
            managed: 0,
            skipped: 0,
            detail: "missing".to_string(),
        }
    }

    fn present(schema: String) -> Self {
        Self {
            present: true,
            schema,
            ok: false,
            managed: 0,
            skipped: 0,
            detail: String::new(),
        }
    }

    fn failed(mut self, detail: &str) -> Self {
        self.ok = false;
        self.detail = detail.to_string();
        self
    }
}

/// Read `<dir>/profile.json` and validate its envelope.
pub fn read_profile_json(dir: &Path) -> JsonFact {
    let path = dir.join(PROFILE_JSON);
    if !path.is_file() {
        return JsonFact::missing();
    }
    let value = match load_json(&path) {
        Ok(value) => value,
        Err(reason) => return JsonFact::present(String::new()).failed(&reason),
    };
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let fact = JsonFact::present(schema.clone());
    if !value.is_object() {
        return fact.failed("not_an_object");
    }
    if contains_nul(&value) {
        return fact.failed("nul_byte");
    }
    match schema.as_str() {
        PROFILE_SCHEMA_V1 => {
            let Some(profile) = value.get("profile").filter(|v| v.is_object()) else {
                return fact.failed("profile_missing");
            };
            if !profile.get("paths").is_some_and(Value::is_object) {
                return fact.failed("paths_missing");
            }
            JsonFact { ok: true, ..fact }
        }
        PROFILE_SCHEMA_V2 => {
            if value.get("profile").is_some_and(Value::is_object) {
                JsonFact { ok: true, ..fact }
            } else {
                fact.failed("profile_missing")
            }
        }
        _ => fact.failed("unsupported_schema"),
    }
}

/// Read `<dir>/installed_mods.json` and count the records an import would
/// consider: managed status, and both identities non-empty. A record without
/// `project_id` or `version_id` cannot be matched to a provider later, so it
/// is skipped instead of invented.
pub fn read_installed_mods(dir: &Path) -> InstalledModsFact {
    let path = dir.join(INSTALLED_MODS_JSON);
    if !path.is_file() {
        return InstalledModsFact::missing();
    }
    let value = match load_json(&path) {
        Ok(value) => value,
        Err(reason) => return InstalledModsFact::present(String::new()).failed(&reason),
    };
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let fact = InstalledModsFact::present(schema.clone());
    if !value.is_object() {
        return fact.failed("not_an_object");
    }
    if contains_nul(&value) {
        return fact.failed("nul_byte");
    }
    if schema != INSTALLED_MODS_SCHEMA_V1 {
        return fact.failed("unsupported_schema");
    }
    let Some(records) = value.get("managed_mods") else {
        return fact.failed("managed_mods_missing");
    };
    let Some(records) = records.as_array() else {
        return fact.failed("managed_mods_not_array");
    };

    let mut managed = 0usize;
    let mut skipped = 0usize;
    for record in records {
        let Some(object) = record.as_object() else {
            skipped += 1;
            continue;
        };
        let project = text(object.get("project_id"));
        let version = text(object.get("version_id"));
        if project.is_empty() || version.is_empty() {
            skipped += 1;
            continue;
        }
        let status = text(object.get("managed_status"));
        let status = if status.is_empty() {
            "managed".to_string()
        } else {
            status.to_lowercase()
        };
        if matches!(
            status.as_str(),
            "external" | "external / unknown" | "unknown"
        ) {
            skipped += 1;
            continue;
        }
        managed += 1;
    }

    InstalledModsFact {
        ok: true,
        managed,
        skipped,
        ..fact
    }
}

fn load_json(path: &Path) -> Result<Value, String> {
    let metadata = std::fs::metadata(path).map_err(|_| "missing".to_string())?;
    if !metadata.is_file() {
        return Err("not_a_file".to_string());
    }
    if metadata.len() > MAX_JSON_BYTES {
        return Err("too_large".to_string());
    }
    let bytes = std::fs::read(path).map_err(|_| "read_failed".to_string())?;
    if bytes.contains(&0) {
        return Err("nul_byte".to_string());
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid_json".to_string())
}

fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn contains_nul(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains('\0'),
        Value::Array(items) => items.iter().any(contains_nul),
        Value::Object(map) => map.values().any(contains_nul),
        _ => false,
    }
}
