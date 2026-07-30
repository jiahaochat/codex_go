mod installer;
mod inventory;
mod process_guard;
mod proxy;
mod secrets;
mod updater;

use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

use chrono::Utc;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use inventory::{CodexStatus, PluginItem, SkillItem};
use secrets::ProxySource;

struct OperationState {
    running: AtomicBool,
}

struct OperationPermit<'a> {
    state: &'a OperationState,
}

impl OperationState {
    fn acquire(&self) -> Result<OperationPermit<'_>, String> {
        self.running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "另一项安装或更新任务正在运行".to_owned())?;
        Ok(OperationPermit { state: self })
    }
}

impl Drop for OperationPermit<'_> {
    fn drop(&mut self) {
        self.state.running.store(false, Ordering::Release);
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyStatus {
    configured: bool,
    core_available: bool,
    source: ProxySource,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSnapshot {
    app_version: String,
    codex: CodexStatus,
    codex_home: String,
    plugins: Vec<PluginItem>,
    skills: Vec<SkillItem>,
    proxy: ProxyStatus,
    warnings: Vec<String>,
    checked_at: String,
}

#[tauri::command]
async fn get_snapshot(app: AppHandle) -> Result<AppSnapshot, String> {
    collect_snapshot(app).await
}

#[tauri::command]
async fn refresh_snapshot(app: AppHandle) -> Result<AppSnapshot, String> {
    collect_snapshot(app).await
}

async fn collect_snapshot(app: AppHandle) -> Result<AppSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || build_snapshot(&app))
        .await
        .map_err(|_| "读取本机 Codex 状态时发生内部错误".to_owned())
}

fn build_snapshot(app: &AppHandle) -> AppSnapshot {
    let inventory = inventory::collect_inventory();
    let mut warnings = inventory
        .warnings
        .into_iter()
        .map(localize_inventory_warning)
        .collect::<Vec<_>>();
    let proxy = current_proxy_status(app, &mut warnings);
    if proxy.configured && !proxy.core_available {
        warnings.push("当前安装包缺少 Xray 核心，暂时无法使用加速安装".to_owned());
    }

    AppSnapshot {
        app_version: app.package_info().version.to_string(),
        codex: inventory.codex,
        codex_home: inventory.codex_home,
        plugins: inventory.plugins,
        skills: inventory.skills,
        proxy,
        warnings,
        checked_at: Utc::now().to_rfc3339(),
    }
}

fn current_proxy_status(app: &AppHandle, warnings: &mut Vec<String>) -> ProxyStatus {
    let core_available = proxy::xray_available(app);
    match secrets::resolve_vless_uri() {
        Ok(Some(resolved)) => ProxyStatus {
            configured: true,
            core_available,
            source: resolved.source,
        },
        Ok(None) => ProxyStatus {
            configured: false,
            core_available,
            source: ProxySource::None,
        },
        Err(message) => {
            warnings.push(format!("下载线路配置无效: {message}"));
            ProxyStatus {
                configured: false,
                core_available,
                source: ProxySource::None,
            }
        }
    }
}

#[tauri::command]
async fn install_codex(app: AppHandle, state: State<'_, OperationState>) -> Result<(), String> {
    let _permit = state.acquire()?;

    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || installer::install(&worker_app))
        .await
        .map_err(|_| "Codex 安装任务意外中止".to_owned())
        .and_then(|result| result);
    if let Err(message) = &result {
        let _ = app.emit(
            "install-progress",
            installer::InstallProgress {
                stage: "error".to_owned(),
                percent: 100,
                message: message.clone(),
            },
        );
    }
    result
}

#[tauri::command]
async fn check_app_update(app: AppHandle) -> Result<Option<updater::UpdateInfo>, String> {
    updater::check(app).await
}

#[tauri::command]
async fn install_app_update(
    app: AppHandle,
    state: State<'_, OperationState>,
) -> Result<(), String> {
    let _permit = state.acquire()?;
    updater::install(app).await
}

#[tauri::command]
fn reveal_path(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !path.exists() {
        return Err("目标位置已不存在".to_owned());
    }

    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("explorer.exe");
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
        if path.is_file() {
            command.arg("/select,").arg(&path);
        } else {
            command.arg(&path);
        }
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        if path.is_file() {
            command.arg("-R");
        }
        command.arg(&path);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(if path.is_file() {
            path.parent().unwrap_or(&path)
        } else {
            &path
        });
        command
    };

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|_| "无法打开目标位置".to_owned())
}

fn localize_inventory_warning(message: String) -> String {
    if let Some(detail) = message.strip_prefix("Unable to query Codex plugins: ") {
        return format!("无法通过 Codex CLI 读取插件列表，已使用本地扫描: {detail}");
    }
    if let Some(detail) = message.strip_prefix("Codex CLI candidates could not be started: ") {
        return format!("检测到 Codex 文件但无法执行: {detail}");
    }
    message
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(OperationState {
            running: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            refresh_snapshot,
            install_codex,
            check_app_update,
            install_app_update,
            reveal_path
        ])
        .run(tauri::generate_context!())
        .expect("failed to run codex_go");
}
