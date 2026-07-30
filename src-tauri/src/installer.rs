use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::{inventory, process_guard::ChildJob, proxy, secrets};

const INSTALLER_SHA256: &str = "391f247de2c70c7e99041979ec02dae7e76be27ac9cfc1dfe7c1eb21d48d8b97";
const MAX_INSTALLER_BYTES: u64 = 2 * 1024 * 1024;
const MAX_INSTALL_OUTPUT_BYTES: u64 = 2 * 1024 * 1024;
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    pub stage: String,
    pub percent: u8,
    pub message: String,
}

pub fn install(app: &AppHandle) -> Result<(), String> {
    emit(app, "preparing", 4, "正在检查安装环境");
    let script = find_installer(app).ok_or_else(|| "安装包中缺少官方 Codex 安装脚本".to_owned())?;
    verify_installer(&script)?;
    let resolved =
        secrets::resolve_vless_uri()?.ok_or_else(|| "请先配置 VLESS 下载线路".to_owned())?;

    emit(app, "proxy", 16, "正在启动本机加速线路");
    let runtime = proxy::start(app, &resolved.uri)?;
    emit(app, "downloading", 38, "正在通过加速线路获取官方文件");

    let command = build_powershell_command(&script, &runtime);
    emit(app, "installing", 58, "正在安装 Codex CLI");
    let output = run_powershell(&command, &runtime.url)?;
    if !output.status.success() {
        let detail = safe_process_error(&output.stdout, &output.stderr, &runtime);
        return Err(if detail.is_empty() {
            "Codex 官方安装程序执行失败".to_owned()
        } else {
            format!("Codex 官方安装程序执行失败: {detail}")
        });
    }

    emit(app, "verifying", 92, "正在验证 Codex 命令");
    let status = inventory::detect_codex_status();
    if !status.installed {
        return Err("安装程序已结束，但未能找到 codex 命令；请重新打开应用后刷新".to_owned());
    }
    emit(app, "complete", 100, "Codex 安装完成");
    Ok(())
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

fn find_installer(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CODEX_GO_INSTALLER_PATH") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("resources/codex/install.ps1"));
        candidates.push(resource_dir.join("codex/install.ps1"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/codex/install.ps1"));
    candidates.into_iter().find(|path| path.is_file())
}

fn verify_installer(path: &Path) -> Result<(), String> {
    let metadata = path
        .metadata()
        .map_err(|_| "无法读取官方 Codex 安装脚本".to_owned())?;
    if metadata.len() == 0 || metadata.len() > MAX_INSTALLER_BYTES {
        return Err("官方 Codex 安装脚本大小异常".to_owned());
    }
    let bytes = fs::read(path).map_err(|_| "无法读取官方 Codex 安装脚本".to_owned())?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != INSTALLER_SHA256 {
        return Err("官方 Codex 安装脚本校验失败".to_owned());
    }
    Ok(())
}

fn build_powershell_command(script: &Path, runtime: &proxy::ProxyRuntime) -> String {
    let script = quote_powershell(&script.to_string_lossy());
    let address = quote_powershell(&runtime.address);
    let username = quote_powershell(&runtime.username);
    let password = quote_powershell(&runtime.password);
    let url = quote_powershell(&runtime.url);
    format!(
        "$ErrorActionPreference='Stop';\
         $proxy='{address}';\
         $proxyPassword=ConvertTo-SecureString '{password}' -AsPlainText -Force;\
         $proxyCredential=New-Object System.Management.Automation.PSCredential('{username}',$proxyPassword);\
         $PSDefaultParameterValues['Invoke-WebRequest:Proxy']=$proxy;\
         $PSDefaultParameterValues['Invoke-WebRequest:ProxyCredential']=$proxyCredential;\
         $PSDefaultParameterValues['Invoke-RestMethod:Proxy']=$proxy;\
         $PSDefaultParameterValues['Invoke-RestMethod:ProxyCredential']=$proxyCredential;\
         $env:HTTP_PROXY='{url}';\
         $env:HTTPS_PROXY='{url}';\
         $env:NO_PROXY='localhost,127.0.0.1';\
         $env:CODEX_NON_INTERACTIVE='1';\
         & '{script}'"
    )
}

fn run_powershell(command: &str, proxy_url: &str) -> Result<Output, String> {
    let executable = powershell_path();
    let encoded = encode_powershell(command);
    let mut stdout_file = tempfile::tempfile().map_err(|_| "无法准备安装日志".to_owned())?;
    let mut stderr_file = tempfile::tempfile().map_err(|_| "无法准备安装日志".to_owned())?;
    let child_stdout = stdout_file
        .try_clone()
        .map_err(|_| "无法准备安装日志".to_owned())?;
    let child_stderr = stderr_file
        .try_clone()
        .map_err(|_| "无法准备安装日志".to_owned())?;
    let mut process = Command::new(executable);
    process
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            &encoded,
        ])
        .env("HTTP_PROXY", proxy_url)
        .env("HTTPS_PROXY", proxy_url)
        .env("NO_PROXY", "localhost,127.0.0.1")
        .env("CODEX_NON_INTERACTIVE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(child_stdout))
        .stderr(Stdio::from(child_stderr));
    hide_window(&mut process);
    let mut child = process
        .spawn()
        .map_err(|_| "无法启动 Windows PowerShell".to_owned())?;
    let _job = ChildJob::attach(&mut child).map_err(|_| "无法约束 Codex 安装进程".to_owned())?;
    let deadline = Instant::now() + INSTALL_TIMEOUT;
    let status = loop {
        match child
            .try_wait()
            .map_err(|_| "无法读取 Codex 安装状态".to_owned())?
        {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Codex 安装超过 30 分钟，任务已停止".to_owned());
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
        .map_err(|_| "无法读取安装日志".to_owned())?;
    let mut bytes = Vec::new();
    file.take(MAX_INSTALL_OUTPUT_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取安装日志".to_owned())?;
    Ok(bytes)
}

fn powershell_path() -> PathBuf {
    if let Ok(system_root) = std::env::var("SystemRoot") {
        let candidate =
            PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("powershell.exe")
}

fn quote_powershell(value: &str) -> String {
    value.replace('\'', "''")
}

fn encode_powershell(command: &str) -> String {
    let bytes: Vec<u8> = command.encode_utf16().flat_map(u16::to_le_bytes).collect();
    STANDARD.encode(bytes)
}

fn safe_process_error(stdout: &[u8], stderr: &[u8], runtime: &proxy::ProxyRuntime) -> String {
    let mut detail = String::from_utf8_lossy(stderr).trim().to_owned();
    if detail.is_empty() {
        detail = String::from_utf8_lossy(stdout).trim().to_owned();
    }
    for secret in [
        &runtime.url,
        &runtime.address,
        &runtime.username,
        &runtime.password,
    ] {
        detail = detail.replace(secret, "[本机代理]");
    }
    detail.chars().take(1200).collect()
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
    fn powershell_encoding_is_utf16le_base64() {
        let encoded = encode_powershell("Write-Output 'ok'");
        let decoded = STANDARD.decode(encoded).unwrap();
        let words: Vec<u16> = decoded
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(String::from_utf16(&words).unwrap(), "Write-Output 'ok'");
    }

    #[test]
    fn powershell_quote_doubles_single_quotes() {
        assert_eq!(
            quote_powershell("C:\\Users\\O'Brien\\a.ps1"),
            "C:\\Users\\O''Brien\\a.ps1"
        );
    }
}
