//! Local jar metadata: strict-priority reader, resource classifier,
//! loader detection, and a (path, size, mtime) scan cache.
//!
//! Informational only (F-091): metadata never proves a Modrinth identity —
//! this module carries no provider/project linkage by construction.

pub mod cache;
pub mod loader;

pub use loader::{detect_loaders, scan_jars_for_loader, Loader};

/// Cap for a single metadata entry (spec §1-1).
pub const MAX_METADATA_BYTES: u64 = 1024 * 1024;

/// Frozen local facts about one archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalModMetadata {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub loaders: Vec<Loader>,
    pub game_versions: Vec<String>,
    pub source_file: std::path::PathBuf,
}

impl LocalModMetadata {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.mod_id
        } else {
            &self.name
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    Mod,
    Shaderpack,
    Datapack,
    Resourcepack,
    Unknown,
}

/// Entry names that mark a mod archive (spec §1-1).
const MOD_METADATA_FILES: &[&str] = &[
    "fabric.mod.json",
    "quilt.mod.json",
    "META-INF/neoforge.mods.toml",
    "META-INF/mods.toml",
];

/// `\` → `/`, strip leading `/`, collapse `//` (spec §2).
fn normalize_entry(name: &str) -> String {
    let mut s = name.replace('\\', "/");
    while s.starts_with('/') {
        s.remove(0);
    }
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    s
}

fn entry_names(archive: &mut zip::ZipArchive<std::fs::File>) -> Vec<String> {
    let len = archive.len();
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        if let Ok(f) = archive.by_index(i) {
            if let Ok(n) = f.name() {
                out.push(normalize_entry(n.as_ref()));
            }
        }
    }
    out
}

/// Capped text read: reject `file_size > MAX`, read `MAX + 1` and reject
/// overflow, decode `utf-8-sig` strictly (spec §1-6).
fn read_zip_text(archive: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let idx = (0..archive.len()).find(|&i| {
        archive
            .by_index(i)
            .ok()
            .and_then(|f| f.name().ok().map(|n| normalize_entry(n.as_ref())))
            .map(|n| n == name)
            .unwrap_or(false)
    })?;
    let file = archive.by_index(idx).ok()?;
    if file.size() > MAX_METADATA_BYTES {
        return None;
    }
    use std::io::Read;
    let mut buf = Vec::new();
    file.take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() as u64 > MAX_METADATA_BYTES {
        return None;
    }
    let bytes: &[u8] = buf.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&buf);
    String::from_utf8(bytes.to_vec()).ok()
}

/// Trim → empty `${…}` → collapse whitespace → truncate 160 chars.
fn clean_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        match rest.find('}') {
            Some(end) => rest = &rest[end + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect()
}

/// Drop empty/`"*` → dedupe → cap 8.
fn clean_versions(versions: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in versions {
        let v = v.trim().to_string();
        if v.is_empty() || v == "*" || out.contains(&v) {
            continue;
        }
        out.push(v);
        if out.len() >= 8 {
            break;
        }
    }
    out
}

#[derive(Debug, serde::Deserialize, Default)]
struct FabricJson {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    depends: std::collections::HashMap<String, OneOrMany>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(v) => v,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct QuiltJson {
    quilt_loader: QuiltLoader,
}

#[derive(Debug, serde::Deserialize, Default)]
struct QuiltLoader {
    #[serde(default)]
    id: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    metadata: QuiltMeta,
    #[serde(default)]
    depends: QuiltDepends,
}

#[derive(Debug, serde::Deserialize, Default)]
struct QuiltMeta {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, serde::Deserialize, Default)]
#[serde(untagged)]
enum QuiltDepends {
    #[default]
    None,
    List(Vec<QuiltDep>),
    Map(std::collections::HashMap<String, QuiltDepValue>),
}

#[derive(Debug, serde::Deserialize, Default)]
struct QuiltDep {
    #[serde(default)]
    id: String,
    #[serde(default)]
    versions: Option<OneOrMany>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum QuiltDepValue {
    Str(String),
    Obj(QuiltDep),
}

fn quilt_minecraft_versions(dep: &QuiltDepends) -> Vec<String> {
    match dep {
        QuiltDepends::None => Vec::new(),
        QuiltDepends::List(list) => list
            .iter()
            .find(|d| d.id == "minecraft")
            .map(|d| match (&d.versions, &d.version) {
                (Some(v), _) => match v {
                    OneOrMany::One(s) => vec![s.clone()],
                    OneOrMany::Many(v) => v.clone(),
                },
                (None, Some(s)) => vec![s.clone()],
                (None, None) => Vec::new(),
            })
            .unwrap_or_default(),
        QuiltDepends::Map(map) => match map.get("minecraft") {
            Some(QuiltDepValue::Str(s)) => vec![s.clone()],
            Some(QuiltDepValue::Obj(d)) => match (&d.versions, &d.version) {
                (Some(OneOrMany::One(s)), _) => vec![s.clone()],
                (Some(OneOrMany::Many(v)), _) => v.clone(),
                (None, Some(s)) => vec![s.clone()],
                (None, None) => Vec::new(),
            },
            None => Vec::new(),
        },
    }
}

fn parse_fabric(text: &str, source: &std::path::Path) -> Option<LocalModMetadata> {
    let mut v: FabricJson = serde_json::from_str(text).ok()?;
    Some(LocalModMetadata {
        mod_id: clean_string(&v.id),
        name: clean_string(&v.name),
        version: clean_string(&v.version),
        description: clean_string(&v.description),
        loaders: vec![Loader::Fabric],
        game_versions: clean_versions(
            v.depends
                .remove("minecraft")
                .map(OneOrMany::into_vec)
                .unwrap_or_default(),
        ),
        source_file: source.to_path_buf(),
    })
}

fn parse_quilt(text: &str, source: &std::path::Path) -> Option<LocalModMetadata> {
    let v: QuiltJson = serde_json::from_str(text).ok()?;
    let l = v.quilt_loader;
    Some(LocalModMetadata {
        mod_id: clean_string(&l.id),
        name: clean_string(&l.metadata.name),
        version: clean_string(&l.version),
        description: clean_string(&l.metadata.description),
        loaders: vec![Loader::Quilt],
        game_versions: clean_versions(quilt_minecraft_versions(&l.depends)),
        source_file: source.to_path_buf(),
    })
}

/// Forge/NeoForge: `mods[0]` identity + the first top-level `dependencies`
/// entry with `modId == "minecraft"` ⇒ raw `versionRange` verbatim.
fn parse_forge_toml(
    text: &str,
    loader: Loader,
    source: &std::path::Path,
) -> Option<LocalModMetadata> {
    let v: toml::Value = toml::from_str(text).ok()?;
    let m0 = v.get("mods")?.as_array()?.first()?;
    let mut versions = Vec::new();
    if let Some(deps) = v.get("dependencies").and_then(|d| d.as_table()) {
        'outer: for node in deps.values() {
            if let Some(entries) = node.as_array() {
                for e in entries {
                    if e.get("modId").and_then(|x| x.as_str()) == Some("minecraft") {
                        if let Some(range) = e.get("versionRange").and_then(|x| x.as_str()) {
                            versions.push(range.to_string());
                        }
                        break 'outer;
                    }
                }
            }
        }
    }
    Some(LocalModMetadata {
        mod_id: clean_string(m0.get("modId")?.as_str()?),
        name: clean_string(m0.get("displayName").and_then(|x| x.as_str()).unwrap_or("")),
        version: clean_string(m0.get("version").and_then(|x| x.as_str()).unwrap_or("")),
        description: clean_string(m0.get("description").and_then(|x| x.as_str()).unwrap_or("")),
        loaders: vec![loader],
        game_versions: clean_versions(versions),
        source_file: source.to_path_buf(),
    })
}

/// Strict first-wins reader (spec §1-2): a present-but-corrupt entry is
/// `None` with no fallback. File-open failures are also `None`.
pub fn read_local_mod_metadata(path: &std::path::Path) -> Option<LocalModMetadata> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    for (entry, loader) in loader::FIRST_WINS {
        if entry_names(&mut archive).iter().any(|n| n == entry) {
            let text = read_zip_text(&mut archive, entry)?;
            return match loader {
                Loader::Fabric => parse_fabric(&text, path),
                Loader::Quilt => parse_quilt(&text, path),
                Loader::Forge | Loader::NeoForge => parse_forge_toml(&text, loader, path),
            };
        }
    }
    None
}

fn has_dir_segment(names: &[String], dir: &str) -> bool {
    names.iter().any(|n| {
        let mut parts: Vec<&str> = n.split('/').collect();
        if parts.len() < 2 {
            return false;
        }
        parts.pop();
        parts.iter().any(|p| p.eq_ignore_ascii_case(dir))
    })
}

fn has_shader_file(names: &[String]) -> bool {
    names.iter().any(|n| {
        let lower = n.to_lowercase();
        has_dir_segment(std::slice::from_ref(n), "shaders")
            && ["fsh", "vsh", "glsl", "csh"]
                .iter()
                .any(|ext| lower.ends_with(&format!(".{ext}")))
    })
}

fn has_pack_mcmeta(names: &[String]) -> bool {
    names
        .iter()
        .any(|n| n == "pack.mcmeta" || n.ends_with("/pack.mcmeta"))
}

/// Classifier over entry names (spec §2): mod → shaderpack → datapack →
/// resourcepack → unknown. Corrupt archives are `Unknown`.
pub fn classify_local_resource_type(path: &std::path::Path) -> ResourceType {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return ResourceType::Unknown,
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => return ResourceType::Unknown,
    };
    let names = entry_names(&mut archive);
    if names
        .iter()
        .any(|n| MOD_METADATA_FILES.contains(&n.as_str()))
    {
        return ResourceType::Mod;
    }
    if has_shader_file(&names) {
        return ResourceType::Shaderpack;
    }
    if has_pack_mcmeta(&names) {
        if has_dir_segment(&names, "data") {
            return ResourceType::Datapack;
        }
        if has_dir_segment(&names, "assets") {
            return ResourceType::Resourcepack;
        }
    }
    ResourceType::Unknown
}

#[cfg(test)]
pub(crate) mod tests;
