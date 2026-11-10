#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod heat_controller;
mod platform_monitor;

use heat_controller::{HeatController, HeatMode, WarmStatus};
use tauri::{Manager, State};

/**
 * @description WarmDesk Tauri application entrypoint and safe low-impact runtime shell.
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-11-10 00:00
 */
#[tauri::command]
fn get_runtime_policy() -> serde_json::Value {
    serde_json::json!({
        "max_intensity": 70,
        "max_expected_cpu_percent": 35,
        "auto_pause_on_system_pressure": true
    })
}

/**
 * @description 返回暖手线程的实时状态和平台保护采样。
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-11-10 14:40
 */
#[tauri::command]
fn get_warm_status(controller: State<'_, HeatController>) -> WarmStatus {
    controller.status()
}

/**
 * @description 启动可取消的 CPU 分片工作负载，按强度和时长执行安全保护。
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-11-10 14:40
 */
#[tauri::command]
fn start_warming(
    intensity: u8,
    mode: HeatMode,
    duration_minutes: u32,
    controller: State<'_, HeatController>,
) -> Result<WarmStatus, String> {
    controller.start(intensity, mode, duration_minutes)
}

/**
 * @description 停止暖手工作线程并等待其退出。
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-11-10 14:40
 */
#[tauri::command]
fn stop_warming(controller: State<'_, HeatController>) -> WarmStatus {
    controller.stop()
}

/**
 * @description 在任务运行中更新暖手强度和模式，不重启工作线程。
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-11-10 14:40
 */
#[tauri::command]
fn update_warm_settings(
    intensity: u8,
    mode: HeatMode,
    controller: State<'_, HeatController>,
) -> Result<WarmStatus, String> {
    controller.update(intensity, mode)
}

/**
 * @description 校验并接收前端主题设置；主题实际持久化由前端本地设置负责。
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-11-10 14:40
 */
#[tauri::command]
fn set_theme(theme: String) -> Result<(), String> {
    match theme.as_str() {
        "light" | "dark" | "system" => Ok(()),
        _ => Err("不支持的主题设置".to_string()),
    }
}

/**
 * @description 返回 WarmDesk 更新检查所需的公开仓库信息，不包含凭证或自动安装逻辑。
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-11-10 11:20
 */
#[tauri::command]
fn get_update_info() -> serde_json::Value {
    serde_json::json!({
        "current_version": env!("CARGO_PKG_VERSION"),
        "repository": "kokotao/warmdesk",
        "mirror_manifest_url": "https://note.lizipro.cn/warmdesk/releases/latest.json",
        "release_api_url": "https://api.github.com/repos/kokotao/warmdesk/releases/latest",
        "release_page_url": "https://github.com/kokotao/warmdesk/releases/latest"
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(HeatController::new())
        .setup(|app| {
            #[cfg(desktop)]
            {
                use tauri::{
                    menu::{Menu, MenuItem},
                    tray::TrayIconBuilder,
                };

                let show_item =
                    MenuItem::with_id(app, "show", "显示 WarmDesk", true, None::<&str>)?;
                let stop_item = MenuItem::with_id(app, "stop", "停止暖手", true, None::<&str>)?;
                let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show_item, &stop_item, &quit_item])?;
                let mut tray = TrayIconBuilder::new()
                    .menu(&menu)
                    .tooltip("WarmDesk · 暖手运行中")
                    .on_menu_event(|app, event| match event.id().as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "stop" => {
                            let _ = app.state::<HeatController>().stop();
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    });
                if let Some(icon) = app.default_window_icon().cloned() {
                    tray = tray.icon(icon);
                }
                tray.build(app)?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_runtime_policy,
            get_update_info,
            get_warm_status,
            start_warming,
            stop_warming,
            update_warm_settings,
            set_theme
        ])
        .run(tauri::generate_context!())
        .expect("error while running WarmDesk");
}
