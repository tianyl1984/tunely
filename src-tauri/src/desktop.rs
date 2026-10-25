use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    AppHandle, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_dialog::DialogExt;

#[derive(Default)]
struct ShutdownState {
    started: AtomicBool,
    complete: AtomicBool,
}

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    app.manage(ShutdownState::default());
    let show = MenuItem::with_id(app, "show-main", "显示主窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Tunely", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &separator, &quit])?;

    #[cfg(target_os = "macos")]
    let icon = tauri::include_image!("icons/tray.png");
    #[cfg(not(target_os = "macos"))]
    let icon = tauri::include_image!("icons/128x128.png");

    TrayIconBuilder::with_id("tunely")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Tunely · SSH 端口转发")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show-main" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn show_main_window(app: &AppHandle) {
    let result = (|| -> tauri::Result<()> {
        if let Some(window) = app.get_webview_window("main") {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Regular)?;
            window.show()?;
            window.unminimize()?;
            window.set_focus()?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("无法显示 Tunely 窗口：{error}");
    }
}

pub fn hide_main_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.hide()?;
        #[cfg(target_os = "macos")]
        app.set_activation_policy(tauri::ActivationPolicy::Accessory)?;
    }
    Ok(())
}

pub fn exit_is_ready(app: &AppHandle) -> bool {
    app.state::<ShutdownState>()
        .complete
        .load(Ordering::Acquire)
}

pub fn request_exit(app: &AppHandle, code: i32) {
    if app
        .state::<ShutdownState>()
        .started
        .swap(true, Ordering::AcqRel)
    {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<crate::commands::AppState>();
        match state.tunnels.shutdown().await {
            Ok(()) => {
                app.state::<ShutdownState>()
                    .complete
                    .store(true, Ordering::Release);
                app.exit(code);
            }
            Err(error) => {
                app.state::<ShutdownState>()
                    .started
                    .store(false, Ordering::Release);
                app.dialog()
                    .message(format!("无法关闭全部通道，请重试退出：{}", error.message))
                    .title("Tunely")
                    .show(|_| {});
            }
        }
    });
}
