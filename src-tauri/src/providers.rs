//! The one place a provider id selects a client (ADR 0004 §2-3).
//!
//! Every command that needs a provider asks here; no other module matches on a
//! provider name. The names themselves live in `core::providers::*::PROVIDER_ID`
//! and are never repeated as literals in this crate.
//!
//! The CurseForge key is read here and never leaves: no DTO, error or log line
//! carries its value (ADR 0004 §2-1-5). Callers that only need the source ask
//! [`key_status`]; the settings screen is that caller.

use mm_core::providers::curseforge::secret::{self, KeySource};
use mm_core::providers::{curseforge, modrinth, CurseForgeClient, ModrinthClient, Provider};

use crate::dto::{CurseForgeKeyDto, ErrorCode, IpcError, KeySourceDto};
use crate::state::AppState;

/// Production selector: the environment is read once, here.
pub fn build(state: &AppState, id: &str) -> Result<Box<dyn Provider>, IpcError> {
    build_with(
        state,
        id,
        std::env::var("CURSEFORGE_API_KEY").ok().as_deref(),
    )
}

/// The selector with the environment key as a parameter, so a test never has
/// to mutate the process environment (the same rule core's secret tests
/// follow).
pub fn build_with(
    state: &AppState,
    id: &str,
    env_key: Option<&str>,
) -> Result<Box<dyn Provider>, IpcError> {
    match id {
        modrinth::PROVIDER_ID => {
            let client = ModrinthClient::new().map_err(IpcError::internal)?;
            Ok(Box::new(client))
        }
        curseforge::PROVIDER_ID => {
            let path = secret::secret_path_in(state.config_dir());
            // A corrupt file is a typed error here too: falling back to the
            // environment would install with a key the status screen never
            // reported (ADR 0004 §2-1-4).
            let (key, _status) =
                secret::load_key(&path, env_key).map_err(|e| IpcError::from_secret(&e))?;
            match key {
                Some(key) => {
                    // `CurseForgeClient::new` keeps the key in memory only;
                    // its error is `Transport`, never the key.
                    let client = CurseForgeClient::new(&key).map_err(IpcError::internal)?;
                    Ok(Box::new(client))
                }
                None => Err(IpcError::new(
                    ErrorCode::ProviderUnauthorized,
                    "no CurseForge API key is configured",
                )),
            }
        }
        other => Err(IpcError::invalid_input(format!(
            "unknown provider {other:?}"
        ))),
    }
}

/// What Settings is allowed to know: the source and the path, never the key.
pub fn key_status(state: &AppState) -> Result<CurseForgeKeyDto, IpcError> {
    key_status_with(state, std::env::var("CURSEFORGE_API_KEY").ok().as_deref())
}

pub fn key_status_with(
    state: &AppState,
    env_key: Option<&str>,
) -> Result<CurseForgeKeyDto, IpcError> {
    let path = secret::secret_path_in(state.config_dir());
    let status = secret::key_status(&path, env_key).map_err(|e| IpcError::from_secret(&e))?;
    Ok(CurseForgeKeyDto {
        source: match status.source {
            KeySource::File => KeySourceDto::File,
            KeySource::Environment => KeySourceDto::Environment,
            KeySource::None => KeySourceDto::None,
        },
        has_key: status.has_key,
        path: path.to_string_lossy().into_owned(),
    })
}

/// Writes the key and answers with the status of the file it just wrote.
///
/// The value goes in and never comes back out: the answer is the same DTO
/// `key_status` produces (ADR 0004 §2-1-5).
pub fn save_key(state: &AppState, key: &str) -> Result<CurseForgeKeyDto, IpcError> {
    let path = secret::secret_path_in(state.config_dir());
    secret::save_key(&path, key).map_err(|e| IpcError::from_secret(&e))?;
    key_status(state)
}

#[cfg(test)]
mod tests;
