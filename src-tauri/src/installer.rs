use std::{
    env,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::{inventory, process_guard::ChildJob, proxy, secrets};

const STORE_PRODUCT_ID: &str = "9PLM9XGG6VKS";
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(3 * 60);
const MAX_OUTPUT_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    pub stage: String,
    pub percent: u8,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUpdateInfo {
    pub available: bool,
}

/// Installs the official Codex Windows desktop app from Microsoft Store.
pub fn install(app: &AppHandle) -> Result<(), String> {
    if inventory::detect_codex_status().installed {
        return update(app);
    }

    emit(app, "preparing", 4, "正在检查 Windows App Installer");
    run_winget(app, "install", "正在安装官方 Codex Windows 桌面端")
}

/// Updates the official Codex Windows desktop app to the newest Store release.
pub fn update(app: &AppHandle) -> Result<(), String> {
    emit(app, "preparing", 4, "正在检查 Codex Windows 更新");
    run_winget(app, "upgrade", "正在更新官方 Codex Windows 桌面端")
}

/// Checks for a Store update without starting an installation.
pub fn check_update(app: &AppHandle) -> Result<CodexUpdateInfo, String> {
    ensure_winget_proxy_enabled()?;
    let uri = secrets::vless_uri()?;
    let runtime = proxy::start(app, uri)?;
    let output = run_winget_command(
        &[
            "list",
            "--id",
            STORE_PRODUCT_ID,
            "--exact",
            "--source",
            "msstore",
            "--upgrade-available",
            "--accept-source-agreements",
            "--disable-interactivity",
        ],
        &runtime,
        CHECK_TIMEOUT,
        "Codex Windows 更新检查超过 3 分钟，任务已停止",
    )?;
    let output_text = process_output_text(&output);

    if has_available_update(&output_text) {
        return Ok(CodexUpdateInfo { available: true });
    }

    if output.status.success() || is_already_current_message(&output_text) {
        return Ok(CodexUpdateInfo { available: false });
    }

    let detail = process_error(&output, &runtime);
    Err(if detail.is_empty() {
        "无法检查 Codex Windows 更新".to_owned()
    } else {
        format!("无法检查 Codex Windows 更新: {detail}")
    })
}

fn run_winget(app: &AppHandle, action: &str, action_message: &str) -> Result<(), String> {
    ensure_winget_proxy_enabled()?;
    let uri = secrets::vless_uri()?;
    let runtime = proxy::start(app, uri)?;
    emit(
        app,
        "downloading",
        28,
        "正在通过 Microsoft Store 获取最新稳定版",
    );
    emit(app, "installing", 55, action_message);
    let output = run_winget_command(
        &[
            action,
            "--id",
            STORE_PRODUCT_ID,
            "--exact",
            "--source",
            "msstore",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--disable-interactivity",
        ],
        &runtime,
        INSTALL_TIMEOUT,
        "Codex Windows 桌面端安装或更新超过 30 分钟，任务已停止",
    )?;

    if !output.status.success() && !is_already_current(&output) {
        let detail = process_error(&output, &runtime);
        if detail.contains("winget.exe") || detail.contains("系统找不到指定的文件") {
            return Err("未找到 Windows App Installer（winget）。请先在 Microsoft Store 更新“应用安装程序”后重试".to_owned());
        }
        return Err(if detail.is_empty() {
            "Microsoft Store 未能完成 Codex 安装或更新".to_owned()
        } else {
            format!("Microsoft Store 未能完成 Codex 安装或更新: {detail}")
        });
    }

    emit(app, "verifying", 92, "正在验证 Codex Windows 桌面端");
    let status = inventory::detect_codex_status();
    if !status.installed {
        return Err("Microsoft Store 任务已结束，但尚未检测到 Codex 桌面端。请稍候刷新，或在 Microsoft Store 中完成安装".to_owned());
    }
    emit(app, "complete", 100, "Codex Windows 桌面端已是最新版本");
    Ok(())
}

fn run_winget_command(
    args: &[&str],
    runtime: &proxy::ProxyRuntime,
    timeout: Duration,
    timeout_message: &str,
) -> Result<Output, String> {
    let mut stdout_file =
        tempfile::tempfile().map_err(|_| "无法准备 Microsoft Store 日志".to_owned())?;
    let mut stderr_file =
        tempfile::tempfile().map_err(|_| "无法准备 Microsoft Store 日志".to_owned())?;
    let child_stdout = stdout_file
        .try_clone()
        .map_err(|_| "无法准备 Microsoft Store 日志".to_owned())?;
    let child_stderr = stderr_file
        .try_clone()
        .map_err(|_| "无法准备 Microsoft Store 日志".to_owned())?;

    let mut command = Command::new(winget_path());
    command
        .args(args)
        .arg("--proxy")
        .arg(&runtime.url)
        .stdin(Stdio::null())
        .stdout(Stdio::from(child_stdout))
        .stderr(Stdio::from(child_stderr));
    hide_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动 winget.exe: {error}"))?;
    let _job =
        ChildJob::attach(&mut child).map_err(|_| "无法约束 Microsoft Store 安装进程".to_owned())?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child
            .try_wait()
            .map_err(|_| "无法读取 Microsoft Store 安装状态".to_owned())?
        {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(timeout_message.to_owned());
            }
            None => thread::sleep(Duration::from_millis(100)),
        }
    };

    Ok(Output {
        status,
        stdout: read_process_output(&mut stdout_file)?,
        stderr: read_process_output(&mut stderr_file)?,
    })
}

fn read_process_output(file: &mut File) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "无法读取 Microsoft Store 日志".to_owned())?;
    let mut bytes = Vec::new();
    file.take(MAX_OUTPUT_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取 Microsoft Store 日志".to_owned())?;
    Ok(bytes)
}

fn is_already_current(output: &Output) -> bool {
    is_already_current_message(&process_output_text(output))
}

fn process_output_text(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn is_already_current_message(message: &str) -> bool {
    let message = message.to_lowercase();
    [
        "no applicable update found",
        "no available upgrade found",
        "no newer package versions are available",
        "no installed package found matching input criteria",
        "no package found matching input criteria",
        "没有适用的更新",
        "找不到可用的升级",
        "没有可用的较新的包版本",
        "没有找到符合输入条件的已安装程序包",
        "没有找到与输入条件匹配的已安装程序包",
    ]
    .iter()
    .any(|expected| message.contains(expected))
}

fn has_available_update(message: &str) -> bool {
    message
        .to_lowercase()
        .contains(&STORE_PRODUCT_ID.to_lowercase())
}

fn process_error(output: &Output, runtime: &proxy::ProxyRuntime) -> String {
    let mut detail = if output.stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout)
    } else {
        String::from_utf8_lossy(&output.stderr)
    }
    .trim()
    .to_owned();
    detail = detail.replace(&runtime.url, "[本机网络服务]");
    detail.chars().take(1200).collect()
}

fn ensure_winget_proxy_enabled() -> Result<(), String> {
    if read_winget_proxy_setting()? {
        return Ok(());
    }

    let winget = winget_path();
    let script = "$process = Start-Process -FilePath $env:CODEX_GO_WINGET_PATH \
        -ArgumentList @('settings','--enable','ProxyCommandLineOptions','--disable-interactivity') \
        -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $process.ExitCode";
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("CODEX_GO_WINGET_PATH", &winget)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_window(&mut command);

    let status = command
        .status()
        .map_err(|_| "无法请求管理员授权以启用 Windows App Installer 的代理支持".to_owned())?;
    if !status.success() || !read_winget_proxy_setting()? {
        return Err(
            "Codex 安装必须通过内置 VLESS 网络连接。请允许管理员授权，以启用 Windows App Installer 的代理支持"
                .to_owned(),
        );
    }
    Ok(())
}

fn read_winget_proxy_setting() -> Result<bool, String> {
    let mut command = Command::new(winget_path());
    command
        .args(["settings", "export", "--disable-interactivity"])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    hide_window(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("无法启动 winget.exe: {error}"))?;
    if !output.status.success() {
        return Err("无法读取 Windows App Installer 代理设置".to_owned());
    }
    parse_winget_proxy_setting(&output.stdout)
}

fn parse_winget_proxy_setting(output: &[u8]) -> Result<bool, String> {
    let settings: serde_json::Value = serde_json::from_slice(output)
        .map_err(|_| "Windows App Installer 返回了无效的设置数据".to_owned())?;
    Ok(settings
        .get("adminSettings")
        .and_then(|value| value.get("ProxyCommandLineOptions"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false))
}

fn winget_path() -> PathBuf {
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        let candidate = PathBuf::from(local_app_data)
            .join("Microsoft")
            .join("WindowsApps")
            .join("winget.exe");
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("winget.exe")
}

fn emit(app: &AppHandle, stage: &str, percent: u8, message: &str) {
    let _ = app.emit(
        "install-progress",
        InstallProgress {
            stage: stage.to_owned(),
            percent,
            message: message.to_owned(),
        },
    );
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_enabled_proxy_command_line_setting() {
        let output = br#"{"adminSettings":{"ProxyCommandLineOptions":true}}"#;
        assert!(parse_winget_proxy_setting(output).unwrap());
    }

    #[test]
    fn treats_missing_proxy_command_line_setting_as_disabled() {
        let output = br#"{"adminSettings":{}}"#;
        assert!(!parse_winget_proxy_setting(output).unwrap());
    }

    #[test]
    fn recognizes_current_winget_no_upgrade_message() {
        let output = "No available upgrade found. No newer package versions are available from the configured sources.";
        assert!(is_already_current_message(output));
    }

    #[test]
    fn recognizes_localized_winget_no_upgrade_message() {
        let output = "找不到可用的升级。配置的源中没有可用的较新的包版本。";
        assert!(is_already_current_message(output));
    }

    #[test]
    fn recognizes_empty_upgrade_filter_result() {
        assert!(is_already_current_message(
            "No installed package found matching input criteria."
        ));
        assert!(is_already_current_message(
            "没有找到与输入条件匹配的已安装程序包。"
        ));
    }

    #[test]
    fn recognizes_store_product_in_upgrade_list() {
        let output = "Codex 9PLM9XGG6VKS 26.7.1 26.7.2 msstore";
        assert!(has_available_update(output));
    }
}
