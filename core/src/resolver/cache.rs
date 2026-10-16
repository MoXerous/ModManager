//! Per-run provider cache (F-058).
//!
//! Owned by one resolve run and dropped with it: no invalidation, no
//! cross-run persistence (`lessons.md` §2). `enabled = false` bypasses
//! every read and write. Diagnostics (`summary()`) are dropped (F-059).

use crate::providers::{HashAlgo, Project, Version};
use std::collections::HashMap;

/// Cache key for one candidate-loader pick, with its channel.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BestKey {
    pub project_id: String,
    pub game_version: String,
    pub loader: String,
    pub allow_alpha: bool,
}

/// The 12-field resolved key (spec §4-1 step 0): every input that can
/// change the outcome is part of the key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ResolveKey {
    pub project_id: String,
    pub slug: String,
    pub project_type: String,
    pub game_version: String,
    pub loader: String,
    pub allow_alpha: bool,
    pub preferred_version_id: String,
    pub preferred_version_number: String,
    pub preferred_file_name: String,
    pub preferred_file_sha1: String,
    pub preferred_game_versions: Vec<String>,
    pub preferred_loaders: Vec<String>,
}

/// Normalised hash-lookup key: algorithm + sorted, lowered, non-empty.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HashKey {
    pub algo: String,
    pub hashes: Vec<String>,
}

impl HashKey {
    pub fn new(algo: HashAlgo, hashes: &[String]) -> Option<Self> {
        let mut hs: Vec<String> = hashes
            .iter()
            .map(|h| h.to_lowercase())
            .filter(|h| !h.trim().is_empty())
            .collect();
        if hs.is_empty() {
            return None;
        }
        hs.sort();
        hs.dedup();
        Some(Self {
            algo: algo.as_str().to_string(),
            hashes: hs,
        })
    }
}

#[derive(Debug, Default)]
pub struct RunCache {
    pub enabled: bool,
    projects: HashMap<String, Project>,
    project_versions: HashMap<(String, String, String), Vec<Version>>,
    hash_versions: HashMap<HashKey, HashMap<String, Version>>,
    best: HashMap<BestKey, Option<Version>>,
    resolved: HashMap<ResolveKey, crate::resolver::graph::ResolvedContent>,
}

impl RunCache {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Default::default()
        }
    }

    pub fn project(&self, id: &str) -> Option<Project> {
        if !self.enabled {
            return None;
        }
        self.projects.get(id).cloned()
    }

    pub fn put_project(&mut self, project: Project) {
        if !self.enabled {
            return;
        }
        self.projects.insert(project.id.clone(), project);
    }

    pub fn project_versions(&self, id: &str, game: &str, loader: &str) -> Option<Vec<Version>> {
        if !self.enabled {
            return None;
        }
        self.project_versions
            .get(&(id.to_string(), game.to_string(), loader.to_string()))
            .cloned()
    }

    pub fn put_project_versions(
        &mut self,
        id: &str,
        game: &str,
        loader: &str,
        versions: Vec<Version>,
    ) {
        if !self.enabled {
            return;
        }
        self.project_versions.insert(
            (id.to_string(), game.to_string(), loader.to_string()),
            versions,
        );
    }

    pub fn hash_versions(&self, key: &HashKey) -> Option<HashMap<String, Version>> {
        if !self.enabled {
            return None;
        }
        self.hash_versions.get(key).cloned()
    }

    pub fn put_hash_versions(&mut self, key: HashKey, map: HashMap<String, Version>) {
        if !self.enabled {
            return;
        }
        self.hash_versions.insert(key, map);
    }

    pub fn best(&self, key: &BestKey) -> Option<Option<Version>> {
        if !self.enabled {
            return None;
        }
        self.best.get(key).cloned()
    }

    pub fn put_best(&mut self, key: BestKey, version: Option<Version>) {
        if !self.enabled {
            return;
        }
        self.best.insert(key, version);
    }

    pub fn resolved(&self, key: &ResolveKey) -> Option<crate::resolver::graph::ResolvedContent> {
        if !self.enabled {
            return None;
        }
        self.resolved.get(key).cloned()
    }

    pub fn put_resolved(
        &mut self,
        key: ResolveKey,
        content: crate::resolver::graph::ResolvedContent,
    ) {
        if !self.enabled {
            return;
        }
        self.resolved.insert(key, content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_key_normalises_order_case_and_empties() {
        let a = HashKey::new(
            HashAlgo::Sha1,
            &["BB".to_string(), "".to_string(), "aa".to_string()],
        )
        .expect("key");
        let b = HashKey::new(HashAlgo::Sha1, &["aa".to_string(), "bb".to_string()]).expect("key");
        assert_eq!(a, b);
        assert!(HashKey::new(HashAlgo::Sha1, &["".to_string()]).is_none());
    }

    #[test]
    fn disabled_cache_never_reads_or_writes() {
        let mut c = RunCache::new(false);
        let key = ResolveKey {
            project_id: "a".to_string(),
            ..Default::default()
        };
        c.put_best(
            BestKey {
                project_id: "a".to_string(),
                game_version: "1.21".to_string(),
                loader: "fabric".to_string(),
                allow_alpha: false,
            },
            None,
        );
        assert_eq!(c.resolved(&key), None);
    }
}
