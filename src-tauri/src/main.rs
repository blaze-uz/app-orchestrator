mod commands;
mod health;
mod models;
mod process_manager;
mod state;
mod storage;

use state::AppState;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let shutdown_started = Arc::new(AtomicBool::new(false));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let config = storage::load_config(app.handle());
            let runtime_processes = storage::load_runtime_processes(app.handle(), &config);
            let state = AppState::new(config, runtime_processes);
            state::set_global_state(state.clone());
            app.manage(state);
            tauri::async_runtime::block_on(process_manager::recover_tracked_processes(
                app.handle().clone(),
                state::app_state(),
            ));
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
            commands::stop_project,
            commands::restart_project,
            commands::restart_failed_processes,
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
            commands::import_config,
            commands::export_config
        ])
        .build(tauri::generate_context!())
        .expect("error while building Local Project Orchestrator");

    let shutdown_started = shutdown_started.clone();
    app.run(move |_app_handle, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            if !shutdown_started.swap(true, Ordering::SeqCst) {
                tauri::async_runtime::block_on(process_manager::shutdown_tracked_processes(
                    _app_handle.clone(),
                    state::app_state(),
                ));
            }
        }
    });
}

fn main() {
    run();
}
