mod commands;
mod desktop;
mod model;
mod ssh;
mod store;
mod tunnel;

#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let store = store::Store::open(app.path().app_data_dir()?)
                .map_err(|error| std::io::Error::other(error.message))?;
            app.manage(commands::AppState {
                store: Arc::new(Mutex::new(store)),
                tunnels: Arc::new(tunnel::TunnelManager::default()),
                lifecycle: tokio::sync::Mutex::new(()),
            });
            desktop::setup(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::load_connections,
            commands::open_local_port,
            commands::save_connection,
            commands::delete_connection,
            commands::test_connection,
            commands::trust_host_key,
            commands::start_forward,
            commands::stop_forward,
            commands::reconnect_forward,
            commands::stop_all_forwards,
            commands::tunnel_statuses,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Tunely");
    app.run(|app, event| match event {
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" => {
            api.prevent_close();
            if let Err(error) = desktop::hide_main_window(app) {
                eprintln!("无法隐藏 Tunely 窗口：{error}");
            }
        }
        tauri::RunEvent::ExitRequested { api, code, .. } => {
            if !desktop::exit_is_ready(app) {
                api.prevent_exit();
                desktop::request_exit(app, code.unwrap_or(0));
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => desktop::show_main_window(app),
        tauri::RunEvent::Exit => {
            let state = app.state::<commands::AppState>();
            tauri::async_runtime::block_on(state.tunnels.shutdown()).ok();
        }
        _ => {}
    });
}
