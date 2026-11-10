#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
        .invoke_handler(tauri::generate_handler![get_runtime_policy, get_update_info])
        .run(tauri::generate_context!())
        .expect("error while running WarmDesk");
}
