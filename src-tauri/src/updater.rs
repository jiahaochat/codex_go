use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

use crate::{proxy, secrets};

const CHECK_TIMEOUT: Duration = Duration::from_secs(90);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub version: String,
    pub notes: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub stage: String,
    pub percent: u8,
    pub message: String,
}

pub async fn check(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let runtime = start_proxy(&app).await?;
    let proxy_url = runtime
        .url
        .parse()
        .map_err(|_| "无法配置软件更新代理".to_owned())?;
    let update = app
        .updater_builder()
        .timeout(CHECK_TIMEOUT)
        .proxy(proxy_url)
        .build()
        .map_err(|_| "无法初始化软件更新器".to_owned())?
        .check()
        .await
        .map_err(|_| "无法检查软件更新".to_owned())?;

    Ok(update.map(|update| UpdateInfo {
        current_version: update.current_version,
        version: update.version,
        notes: update.body,
        published_at: update.date.map(|date| date.to_string()),
    }))
}

pub async fn install(app: AppHandle) -> Result<(), String> {
    emit(&app, "proxy", 5, "正在准备网络连接");
    let runtime = start_proxy(&app).await?;
    let proxy_url = runtime
        .url
        .parse()
        .map_err(|_| "无法配置软件更新代理".to_owned())?;

    emit(&app, "checking", 12, "正在确认最新版本");
    let mut update = app
        .updater_builder()
        .timeout(CHECK_TIMEOUT)
        .proxy(proxy_url)
        .build()
        .map_err(|_| "无法初始化软件更新器".to_owned())?
        .check()
        .await
        .map_err(|_| "无法检查软件更新".to_owned())?
        .ok_or_else(|| "当前已经是最新版本".to_owned())?;
    update.timeout = Some(INSTALL_TIMEOUT);

    let progress_app = app.clone();
    let finished_app = app.clone();
    let mut downloaded = 0u64;
    emit(&app, "downloading", 18, "正在下载签名更新包");
    update
        .download_and_install(
            move |chunk_length, content_length| {
                downloaded = downloaded.saturating_add(chunk_length as u64);
                let percent = content_length
                    .filter(|total| *total > 0)
                    .map(|total| {
                        18u64
                            .saturating_add(downloaded.saturating_mul(72) / total)
                            .min(90) as u8
                    })
                    .unwrap_or(45);
                emit(&progress_app, "downloading", percent, "正在下载签名更新包");
            },
            move || emit(&finished_app, "verifying", 94, "正在验证更新签名"),
        )
        .await
        .map_err(|_| "更新下载、验签或安装失败".to_owned())?;

    emit(&app, "complete", 100, "更新安装完成，正在重启");
    app.restart();
}

async fn start_proxy(app: &AppHandle) -> Result<proxy::ProxyRuntime, String> {
    let uri = secrets::vless_uri()?;
    let proxy_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || proxy::start(&proxy_app, uri))
        .await
        .map_err(|_| "启动软件更新代理时发生内部错误".to_owned())?
}

fn emit(app: &AppHandle, stage: &str, percent: u8, message: &str) {
    let _ = app.emit(
        "app-update-progress",
        UpdateProgress {
            stage: stage.to_owned(),
            percent,
            message: message.to_owned(),
        },
    );
}
