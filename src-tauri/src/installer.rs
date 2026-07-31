use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::{inventory, process_guard::ChildJob, proxy, secrets};

const STORE_PRODUCT_ID: &str = "9PLM9XGG6VKS";
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_OUTPUT_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    pub stage: String,
    pub percent: u8,
    pub message: String,
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

fn run_winget(app: &AppHandle, action: &str, action_message: &str) -> Result<(), String> {
    let uri = secrets::vless_uri()?;
    let runtime = proxy::start(app, uri)?;
    emit(
        app,
        "downloading",
        28,
        "正在通过 Microsoft Store 获取最新稳定版",
    );
    emit(app, "installing", 55, action_message);
    let output = run_winget_command(action, &runtime)?;

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

fn run_winget_command(action: &str, runtime: &proxy::ProxyRuntime) -> Result<Output, String> {
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

    let mut command = Command::new("winget.exe");
    command
        .args([
            action,
            "--id",
            STORE_PRODUCT_ID,
            "--exact",
            "--source",
            "msstore",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--disable-interactivity",
        ])
        .arg("--proxy")
        .arg(&runtime.url)
        .env("HTTP_PROXY", &runtime.url)
        .env("HTTPS_PROXY", &runtime.url)
        .env("ALL_PROXY", &runtime.url)
        .env("NO_PROXY", "localhost,127.0.0.1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(child_stdout))
        .stderr(Stdio::from(child_stderr));
    hide_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动 winget.exe: {error}"))?;
    let _job =
        ChildJob::attach(&mut child).map_err(|_| "无法约束 Microsoft Store 安装进程".to_owned())?;
    let deadline = Instant::now() + INSTALL_TIMEOUT;
    let status = loop {
        match child
            .try_wait()
            .map_err(|_| "无法读取 Microsoft Store 安装状态".to_owned())?
        {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Codex Windows 桌面端安装或更新超过 30 分钟，任务已停止".to_owned());
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
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_lowercase();
    text.contains("no applicable update found") || text.contains("没有适用的更新")
}

fn process_error(output: &Output, runtime: &proxy::ProxyRuntime) -> String {
    let mut detail = if output.stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout)
    } else {
        String::from_utf8_lossy(&output.stderr)
    }
    .trim()
    .to_owned();
    for secret in [
        &runtime.url,
        &runtime.address,
        &runtime.username,
        &runtime.password,
    ] {
        detail = detail.replace(secret, "[本机网络服务]");
    }
    detail.chars().take(1200).collect()
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
