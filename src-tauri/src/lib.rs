#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod heat_controller;
mod platform_monitor;

use heat_controller::{HeatController, HeatMode, WarmStatus};
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::PathBuf,
    time::Duration,
};
use tauri::{Emitter, Manager, State};

const TRUSTED_DOWNLOAD_HOSTS: &[&str] = &[
    "note.lizipro.cn",
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];

fn is_allowed_download_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|host| TRUSTED_DOWNLOAD_HOSTS.contains(&host))
}

fn sanitize_download_filename(value: &str) -> Option<String> {
    let name = value.trim();
    if name.is_empty()
        || name.len() > 180
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || name.contains('?')
        || name.contains('#')
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._+-".contains(character))
    {
        return None;
    }
    Some(name.to_string())
}

#[derive(Clone, serde::Serialize)]
struct DownloadProgress {
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    percent: Option<u8>,
}

struct PartialFileCleanup {
    path: PathBuf,
    keep: bool,
}

impl Drop for PartialFileCleanup {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/**
 * @description 下载并校验更新安装包，写入系统 Downloads 目录并报告实时进度。
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-11-10 18:20
 */
#[tauri::command]
async fn download_update(
    app: tauri::AppHandle,
    url: String,
    filename: String,
    expected_size: Option<u64>,
    expected_sha256: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        download_update_blocking(app, url, filename, expected_size, expected_sha256)
    })
    .await
    .map_err(|error| format!("下载任务中断：{error}"))?
}

fn download_update_blocking(
    app: tauri::AppHandle,
    url: String,
    filename: String,
    expected_size: Option<u64>,
    expected_sha256: Option<String>,
) -> Result<String, String> {
    if !is_allowed_download_url(&url) {
        return Err("更新地址不受信任，仅支持 WarmDesk 镜像或 GitHub HTTPS 地址".to_string());
    }
    let filename =
        sanitize_download_filename(&filename).ok_or_else(|| "更新文件名无效".to_string())?;
    let expected_sha256 = expected_sha256
        .filter(|value| {
            value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
        })
        .map(|value| value.to_ascii_lowercase());
    let download_dir = app
        .path()
        .download_dir()
        .map_err(|error| format!("无法定位系统下载目录：{error}"))?;
    fs::create_dir_all(&download_dir).map_err(|error| format!("无法创建下载目录：{error}"))?;
    let destination = unique_download_path(&download_dir, &filename);
    let partial = destination.with_extension(format!(
        "{}.part",
        destination
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("download")
    ));
    let mut partial_cleanup = PartialFileCleanup {
        path: partial.clone(),
        keep: false,
    };
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(1800))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.stop()
            } else if is_allowed_download_url(attempt.url().as_str()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|error| format!("创建下载连接失败：{error}"))?;
    let mut response = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, "WarmDesk-Updater/0.2")
        .send()
        .map_err(|error| format!("下载连接失败：{error}"))?;
    if !response.status().is_success() || !is_allowed_download_url(response.url().as_str()) {
        return Err(format!("更新下载地址响应无效：{}", response.status()));
    }
    let total_bytes = response.content_length().or(expected_size);
    let mut output =
        File::create(&partial).map_err(|error| format!("无法写入下载文件：{error}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded_bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| format!("下载中断：{error}"))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("保存下载文件失败：{error}"))?;
        hasher.update(&buffer[..read]);
        downloaded_bytes += read as u64;
        let percent = total_bytes
            .filter(|total| *total > 0)
            .map(|total| ((downloaded_bytes.saturating_mul(100) / total).min(100)) as u8);
        let _ = app.emit(
            "download-progress",
            DownloadProgress {
                downloaded_bytes,
                total_bytes,
                percent,
            },
        );
    }
    output
        .sync_all()
        .map_err(|error| format!("无法完成下载文件写入：{error}"))?;
    if let Some(expected) = expected_size {
        if downloaded_bytes != expected {
            return Err(format!(
                "下载文件大小校验失败：收到 {downloaded_bytes} 字节，预期 {expected} 字节"
            ));
        }
    }
    if let Some(expected) = expected_sha256 {
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected {
            return Err("下载文件 SHA-256 校验失败".to_string());
        }
    }
    fs::rename(&partial, &destination).map_err(|error| format!("无法提交下载文件：{error}"))?;
    partial_cleanup.keep = true;
    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            downloaded_bytes,
            total_bytes: Some(downloaded_bytes),
            percent: Some(100),
        },
    );
    Ok(destination.to_string_lossy().into_owned())
}

fn unique_download_path(download_dir: &std::path::Path, filename: &str) -> PathBuf {
    let candidate = download_dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let path = std::path::Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("WarmDesk");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    for index in 1..1000 {
        let name = if extension.is_empty() {
            format!("{stem} ({index})")
        } else {
            format!("{stem} ({index}).{extension}")
        };
        let candidate = download_dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    download_dir.join(format!("{stem}-download.{extension}"))
}

#[cfg(test)]
mod update_download_tests {
    use super::{is_allowed_download_url, sanitize_download_filename, PartialFileCleanup};
    use std::fs::File;

    #[test]
    fn download_url_must_use_https_and_trusted_hosts() {
        assert!(is_allowed_download_url(
            "https://note.lizipro.cn/warmdesk/releases/0.2.2/package.dmg"
        ));
        assert!(is_allowed_download_url(
            "https://github.com/kokotao/warmdesk/releases/download/v0.2.2/package.msi"
        ));
        assert!(is_allowed_download_url(
            "https://release-assets.githubusercontent.com/package"
        ));
        assert!(!is_allowed_download_url(
            "http://note.lizipro.cn/package.dmg"
        ));
        assert!(!is_allowed_download_url(
            "https://attacker.example/package.dmg"
        ));
        assert!(!is_allowed_download_url(
            "https://github.com.attacker.example/package"
        ));
    }

    #[test]
    fn download_filename_is_restricted_to_safe_asset_names() {
        assert_eq!(
            sanitize_download_filename("WarmDesk_0.2.2_x64.dmg").as_deref(),
            Some("WarmDesk_0.2.2_x64.dmg")
        );
        assert!(sanitize_download_filename("../WarmDesk.dmg").is_none());
        assert!(sanitize_download_filename("WarmDesk.exe?run=1").is_none());
    }

    #[test]
    fn partial_download_is_removed_when_operation_exits() {
        let path = std::env::temp_dir().join(format!(
            "warmdesk-partial-cleanup-{}.part",
            std::process::id()
        ));
        File::create(&path).expect("create partial fixture");
        let cleanup = PartialFileCleanup {
            path: path.clone(),
            keep: false,
        };
        drop(cleanup);
        assert!(!path.exists());
    }
}

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
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
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
            download_update,
            get_warm_status,
            start_warming,
            stop_warming,
            update_warm_settings,
            set_theme
        ])
        .run(tauri::generate_context!())
        .expect("error while running WarmDesk");
}
