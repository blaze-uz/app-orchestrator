mod commands;
mod health;
mod mediaguard_preset;
mod models;
mod process_manager;
mod state;
mod storage;

use state::AppState;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{Manager, WindowEvent};

fn run_startup_step<F: FnOnce()>(name: &str, step: F) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(step)) {
        let message = if let Some(msg) = payload.downcast_ref::<&str>() {
            (*msg).to_string()
        } else if let Some(msg) = payload.downcast_ref::<String>() {
            msg.clone()
        } else {
            "unknown panic".to_string()
        };
        eprintln!("[startup] {name} panicked: {message}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let shutdown_started = Arc::new(AtomicBool::new(false));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let config = storage::load_config(app.handle());
            let recent_logs =
                storage::load_recent_logs(app.handle(), process_manager::log_history_since());
            let _ = storage::prune_log_history(app.handle(), process_manager::log_history_since());
            let runtime_processes = storage::load_runtime_processes(app.handle(), &config);
            let state = AppState::new(config, runtime_processes, recent_logs);
            state::set_global_state(state.clone());
            app.manage(state);
            process_manager::start_log_history_pruner(app.handle().clone(), state::app_state());
            run_startup_step("recover_tracked_processes", || {
                tauri::async_runtime::block_on(process_manager::recover_tracked_processes(
                    app.handle().clone(),
                    state::app_state(),
                ))
            });
            run_startup_step("sync_external_processes", || {
                tauri::async_runtime::block_on(process_manager::sync_external_processes(
                    app.handle().clone(),
                    state::app_state(),
                ))
            });
            run_startup_step("start_marked_projects_on_launch", || {
                tauri::async_runtime::block_on(process_manager::start_marked_projects_on_launch(
                    app.handle().clone(),
                    state::app_state(),
                ))
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::list_workspaces,
            commands::create_workspace,
            commands::update_workspace,
            commands::delete_workspace,
            commands::list_projects,
            commands::create_project,
            commands::update_project,
            commands::delete_project,
            commands::get_project_detail,
            commands::create_process_definition,
            commands::update_process_definition,
            commands::delete_process_definition,
            commands::list_processes_by_project,
            commands::start_process,
            commands::stop_process,
            commands::restart_process,
            commands::start_project,
            commands::start_auto_start_processes,
            commands::stop_project,
            commands::restart_project,
            commands::restart_failed_processes,
            commands::list_external_project_processes,
            commands::stop_external_process,
            commands::get_runtime_state,
            commands::get_all_runtime_states,
            commands::get_log_history,
            commands::subscribe_logs,
            commands::unsubscribe_logs,
            commands::clear_log_history,
            commands::export_logs,
            commands::run_health_check,
            commands::get_health_summary,
            commands::get_dashboard_summary,
            commands::open_project_folder_in_finder,
            commands::reveal_log_file_in_finder,
            commands::validate_project_path,
            commands::detect_ports_in_use,
            commands::update_settings,
            commands::apply_media_guard_preset,
            commands::import_config,
            commands::export_config
        ])
        .build(tauri::generate_context!())
        .expect("error while building App Orchestrator");

    let shutdown_started = shutdown_started.clone();
    app.run(move |_app_handle, event| match event {
        tauri::RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } => {
            api.prevent_close();
            if let Some(window) = _app_handle.get_webview_window(&label) {
                let _ = window.hide();
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            if let Some(window) = _app_handle.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        tauri::RunEvent::ExitRequested { .. } => {
            if !shutdown_started.swap(true, Ordering::SeqCst) {
                tauri::async_runtime::block_on(process_manager::shutdown_tracked_processes(
                    _app_handle.clone(),
                    state::app_state(),
                ));
            }
        }
        _ => {}
    });
}

fn main() {
    run();
}
