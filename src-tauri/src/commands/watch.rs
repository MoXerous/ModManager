//! F-168: the content watcher commands.
//!
//! The webview asks to watch one instance and gets a typed event back when
//! the watcher sees the content move. The rules (debounce, suppression of the
//! app's own writes, read-only watching) live in `mm_core::watcher`; this
//! module only carries the signal to the webview and tears the watcher down
//! when the instance changes.

use std::path::PathBuf;

use tauri::Emitter as _;

use crate::dto::{ContentStaleDto, ErrorCode, IpcError};
use crate::state::AppState;

/// The typed event name the frontend listens for.
pub const CONTENT_STALE_EVENT: &str = "content://stale";

#[tauri::command]
pub async fn watch_instance_content(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    instance_root: String,
) -> Result<(), IpcError> {
    let root = crate::commands::resolve_dir(&instance_root)?;
    let emitter = app.clone();
    start_content_watch(state.inner(), root, move |dto| {
        // An emit against a closed window is advisory: the watcher is torn
        // down with the app anyway.
        let _ = emitter.emit(CONTENT_STALE_EVENT, dto);
    })
}

#[tauri::command]
pub async fn unwatch_instance_content(state: tauri::State<'_, AppState>) -> Result<(), IpcError> {
    state.stop_content_watch();
    Ok(())
}

/// The body of [`watch_instance_content`], with the emit as a parameter so a
/// test can record signals without a webview.
///
/// Which folders are watched: the ones `scan_instance` reads — `mods/`
/// (the scan's only recursive walk, `disabled_mods/` included). When `mods/`
/// does not exist yet (a fresh instance), the instance root itself is
/// watched instead, so the first jar ever dropped in is still seen; the
/// relevance rule then keeps everything outside the content folders
/// (`logs/`, `config/`, `screenshots/`) from signalling staleness, and the
/// suppression rule keeps the app's own writes quiet.
pub(crate) fn start_content_watch(
    state: &AppState,
    root: PathBuf,
    emit: impl Fn(ContentStaleDto) + Send + 'static,
) -> Result<(), IpcError> {
    let mods = root.join("mods");
    let watched = if mods.is_dir() {
        vec![mods]
    } else {
        vec![root.clone()]
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let config = mm_core::watcher::WatchConfig {
        // F-168: only the content folders may signal, whichever directory is
        // watched. `disabled_mods/` lives inside `mods/`, but the prefix is
        // stated so the rule holds even if the scan's shape ever widens.
        relevant_under: vec![root.join("mods"), root.join("disabled_mods")],
        ..mm_core::watcher::WatchConfig::default()
    };
    let handle = mm_core::watcher::watch(watched, config, state.own_writes().clone(), tx)
        .map_err(|e| IpcError::new(ErrorCode::Io, e))?;
    let dto_root = root.to_string_lossy().to_string();
    // The bridge thread: one signal → one typed event. It ends when the
    // watcher ends, because the watcher owns the sender.
    std::thread::Builder::new()
        .name("content-stale-bridge".to_string())
        .spawn(move || {
            for _ in rx {
                emit(ContentStaleDto {
                    instance_root: dto_root.clone(),
                });
            }
        })
        .map_err(IpcError::io)?;
    state.replace_content_watch(Some(crate::state::ContentWatchSlot {
        handle: std::sync::Arc::new(handle),
    }));
    Ok(())
}

#[cfg(test)]
mod tests;
