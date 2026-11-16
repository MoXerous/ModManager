mod commands;
mod dto;
mod progress;
mod providers;
mod state;

use state::{AppState, StartupFailure, STARTUP_FAILURE_EXIT_CODE};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt as _;

/// F-139: builds the window the config declares with `"create": false`.
///
/// Tauri creates config windows *before* `.setup`, so a window declared there
/// would exist over a database the gate has not approved yet. Declaring it
/// with `"create": false` and building it here keeps that order: no UI until
/// the store is known good.
fn create_main_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let config = app
        .config()
        .app
        .windows
        .first()
        .ok_or("tauri.conf.json declares no window")?;
    tauri::WebviewWindowBuilder::from_config(app.handle(), config)?.build()?;
    Ok(())
}

/// F-139: says why the app will not start, then leaves with code 2.
///
/// A native dialog, because there is no window to put a message in: this path
/// runs before any window is created. `show` is the non-blocking form on
/// purpose — `blocking_show` must not run on the main thread — and the exit
/// happens when the dialog is dismissed.
fn report_startup_failure(app: &tauri::App, failure: StartupFailure) {
    // F-138: the sentence and the cause have different audiences. The log
    // keeps the cause (sanitized); the dialog gets the sentence only.
    mm_core::logging::error(failure.diagnostic());
    let handle = app.handle().clone();
    app.dialog()
        .message(failure.dialog_text())
        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
        .show(move |_| handle.exit(STARTUP_FAILURE_EXIT_CODE));
}

/// F-138: starts the activity log for the whole process, before the gate.
///
/// It has to exist *before* `.setup`, because the gate inside that hook is
/// the failure it is here to record: a refused startup never gets a window to
/// report in, so a cause that is not written now leaves with the process. The
/// guard must live as long as the app: dropping it flushes the log.
///
/// Failing to start the log is not a startup failure of its own — the dialog
/// still has its sentence — so the answer is `None`, not an error.
fn start_logging() -> Option<mm_core::logging::LogGuard> {
    let paths = mm_core::paths::Paths::resolve().ok()?;
    mm_core::logging::init(paths.logs_dir()).ok()
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> i32 {
    // F-139: the log first, because the very next thing that happens is the
    // gate this log exists for.
    let _log = start_logging();
    let built = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // The platform directories are fixed for the process and the
            // database lives under machine-local data so a roaming-synced WAL
            // cannot corrupt it (paths.rs, spec §1-8).
            //
            // F-139: the gate comes first, and it is not `?`. Letting the
            // error out through `.setup` ends in Tauri's generic setup panic:
            // no sentence the user can read and no exit code of its own.
            match AppState::open() {
                Ok(state) => {
                    app.manage(state);
                    // F-139: a window the window layer cannot build is the
                    // same story as a refused database: a `?` here would end
                    // in that panic, so it takes the dialog + exit-2 path.
                    if let Err(error) = create_main_window(app) {
                        report_startup_failure(app, StartupFailure::window_failed(error.as_ref()));
                    }
                }
                Err(failure) => report_startup_failure(app, failure),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::search_projects,
            commands::build_install_plan,
            commands::scan_instance,
            commands::check_instance_updates,
            commands::preview_toggle,
            commands::execute_toggle,
            commands::preview_remove_content,
            commands::execute_remove_content,
            commands::list_instances,
            commands::plan_create_instance,
            commands::execute_create_instance,
            commands::link_instance,
            commands::list_pending_creates,
            commands::finish_create,
            commands::abandon_create,
            commands::open_instance_folder,
            commands::app_paths,
            commands::open_app_folder,
            commands::execute_install,
            commands::cancel_active_install,
            commands::list_download_history,
            commands::clear_download_history,
            commands::curseforge_key_status,
            commands::save_curseforge_key,
            commands::legacy::scan_legacy_import,
            commands::legacy::plan_legacy_import,
            commands::legacy::execute_legacy_import,
            // The pack commands are the module's own: `generate_handler!`
            // resolves the macro support next to each function, so the path
            // has to be the module that defines them.
            commands::pack::plan_modpack_install,
            commands::pack::execute_modpack_install,
            commands::pack::plan_modpack_export,
            commands::pack::write_modpack_export,
            commands::pack::list_export_folders,
            commands::pack::pending_pack_recoveries,
            commands::pack::recover_pack_apply,
            commands::pack::dismiss_pack_apply,
        ])
        .build(tauri::generate_context!());

    match built {
        // `run_return` gives the event loop's exit code back instead of
        // ending the process itself, so a refused startup can leave with 2.
        Ok(app) => app.run_return(|_, _| {}),
        // Nothing to show a dialog on: the window layer itself failed. This
        // is not the F-139 path (the database gate never ran).
        Err(error) => record_build_failure(&error.to_string()),
    }
}

/// F-139: the app never got built. There is no `App`, so no dialog — and a
/// release Windows build has no console (`windows_subsystem = "windows"`), so
/// the `eprintln!` this replaces left a silent exit behind. The log is the
/// channel that is left; recording must not depend on it having been
/// initialised, because an unresolvable data directory is one more way to
/// reach this arm.
///
/// The code stays a generic failure: 2 means "the database gate refused",
/// and this path never ran the gate.
fn record_build_failure(error: &str) -> i32 {
    mm_core::logging::error(format_args!(
        "ModManager cannot start: {}",
        mm_core::logging::sanitize(error)
    ));
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-139: a failed build is recorded and answers a plain failure.
    ///
    /// The recording has to be safe with no subscriber installed (a failed
    /// build can be caused by the directory the log would live in), and the
    /// code must not borrow the refused-startup one: a script that reads 2
    /// wants to say "the database was refused", which never happened here.
    #[test]
    fn spec_f139_a_failed_build_is_recorded_and_leaves_a_plain_failure() {
        let code = record_build_failure(
            r"webview2 not found under C:\Users\Alice\AppData\Local\ModManager",
        );
        assert_eq!(code, 1, "a failed build is not the refused startup");
        assert_ne!(code, STARTUP_FAILURE_EXIT_CODE);
    }
}
