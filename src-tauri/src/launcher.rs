use crate::inventory::CodexStatus;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    net::TcpListener,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[cfg(all(test, windows))]
use std::os::windows::process::CommandExt;

const DEBUG_PORT: u16 = 9230;
#[cfg(all(test, windows))]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CDP_TIMEOUT: Duration = Duration::from_secs(3);
const LAUNCH_ATTEMPTS: usize = 32;
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(4);

static WATCHDOG_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexRuntimeStatus {
    pub state: CodexRuntimeState,
    pub running: bool,
    pub managed: bool,
    pub restart_required: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchProgress {
    pub stage: &'static str,
    pub message: String,
}

#[derive(Default)]
struct DesktopProcesses {
    process_ids: Vec<u32>,
    visible_window: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CodexRuntimeState {
    Stopped,
    Unmanaged,
    Managed,
}

fn classify_runtime(
    process_running: bool,
    visible_window: bool,
    managed: bool,
) -> CodexRuntimeStatus {
    let running = managed || (process_running && visible_window);
    CodexRuntimeStatus {
        state: if managed {
            CodexRuntimeState::Managed
        } else if running {
            CodexRuntimeState::Unmanaged
        } else {
            CodexRuntimeState::Stopped
        },
        running,
        managed,
        restart_required: running && !managed,
    }
}

#[derive(Clone, Debug, Deserialize)]
struct CdpTarget {
    #[serde(rename = "type")]
    target_type: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default, rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: Option<String>,
}

pub fn runtime_status(codex: &CodexStatus) -> CodexRuntimeStatus {
    if !codex.installed {
        return stopped_status();
    }

    let executable = codex.path.as_deref().and_then(codex_executable);
    let processes = executable
        .as_deref()
        .map(desktop_processes)
        .unwrap_or_default();
    let managed = primary_target_blocking().is_some();

    classify_runtime(
        !processes.process_ids.is_empty(),
        processes.visible_window,
        managed,
    )
}

pub async fn launch(app: &AppHandle, codex: &CodexStatus) -> Result<CodexRuntimeStatus, String> {
    emit_progress(app, "starting", "正在启动 Codex Windows 桌面端");
    if !codex.installed {
        return Err("尚未安装 Codex Windows 桌面端".to_owned());
    }

    let app_version = app.package_info().version.to_string();
    let executable = codex
        .path
        .as_deref()
        .and_then(codex_executable)
        .ok_or_else(|| "无法定位官方 Codex Windows 桌面端的启动文件".to_owned())?;
    let processes = desktop_processes(&executable);
    let managed = primary_target_async().await.is_some();
    let current = classify_runtime(
        !processes.process_ids.is_empty(),
        processes.visible_window,
        managed,
    );

    if current.managed {
        emit_progress(app, "complete", "Codex 已运行");
        start_injection_task(app.clone(), app_version);
        return Ok(current);
    }

    if !processes.process_ids.is_empty() {
        emit_progress(
            app,
            "cleaning",
            "正在关闭当前 Codex 并通过 Codex Go 重新启动",
        );
        let cleanup_executable = executable.clone();
        tauri::async_runtime::spawn_blocking(move || {
            terminate_background_processes(&cleanup_executable, &processes)
        })
        .await
        .map_err(|_| "清理 Codex 后台进程时发生内部错误".to_owned())??;
    }

    if TcpListener::bind(("127.0.0.1", DEBUG_PORT)).is_err() {
        return Err(format!(
            "Codex Go 启动端口 {DEBUG_PORT} 已被其他程序占用，请关闭占用程序后重试"
        ));
    }

    let arguments = format!(
        "--remote-debugging-port={DEBUG_PORT} --remote-allow-origins=http://127.0.0.1:{DEBUG_PORT}"
    );
    let activation_id = codex
        .path
        .as_deref()
        .and_then(packaged_app_user_model_id)
        .ok_or_else(|| "无法解析 Codex Windows 应用标识".to_owned())?;
    #[cfg(windows)]
    let process_id = activate_packaged_app(&activation_id, &arguments).await?;
    #[cfg(not(windows))]
    return Err("Codex Windows 打包应用激活仅支持 Windows".to_owned());

    #[cfg(windows)]
    emit_progress(
        app,
        "complete",
        &format!("Codex 已通过 Codex Go 启动（PID {process_id}）"),
    );
    start_injection_task(app.clone(), app_version);
    Ok(CodexRuntimeStatus {
        state: CodexRuntimeState::Managed,
        running: true,
        managed: true,
        restart_required: false,
    })
}

fn emit_progress(app: &AppHandle, stage: &'static str, message: &str) {
    let _ = app.emit(
        "codex-launch-progress",
        LaunchProgress {
            stage,
            message: message.to_owned(),
        },
    );
}

fn stopped_status() -> CodexRuntimeStatus {
    CodexRuntimeStatus {
        state: CodexRuntimeState::Stopped,
        running: false,
        managed: false,
        restart_required: false,
    }
}

fn codex_executable(install_location: &str) -> Option<PathBuf> {
    let root = PathBuf::from(install_location);
    let app_dir = if root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("app"))
    {
        root
    } else {
        root.join("app")
    };

    ["ChatGPT.exe", "Codex.exe"]
        .into_iter()
        .map(|name| app_dir.join(name))
        .find(|path| path.is_file())
}

fn packaged_app_user_model_id(install_location: &str) -> Option<String> {
    let package_name = Path::new(install_location).file_name()?.to_str()?;
    let identity = package_name.split('_').next()?.trim();
    let publisher_id = package_name.rsplit("__").next()?.split('_').next()?.trim();
    if identity.is_empty() || publisher_id.is_empty() {
        return None;
    }
    Some(format!("{identity}_{publisher_id}!App"))
}

#[cfg(windows)]
async fn activate_packaged_app(app_user_model_id: &str, arguments: &str) -> Result<u32, String> {
    let app_user_model_id = app_user_model_id.to_owned();
    let arguments = arguments.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        activate_packaged_app_blocking(&app_user_model_id, &arguments)
    })
    .await
    .map_err(|_| "Windows 打包应用激活任务异常中止".to_owned())?
}

#[cfg(windows)]
fn activate_packaged_app_blocking(app_user_model_id: &str, arguments: &str) -> Result<u32, String> {
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        ApplicationActivationManager, IApplicationActivationManager, ACTIVATEOPTIONS,
    };

    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let should_uninitialize = initialized.is_ok();
        if !should_uninitialize {
            const RPC_E_CHANGED_MODE: i32 = -2147417850;
            if initialized.0 != RPC_E_CHANGED_MODE {
                return Err(format!(
                    "COM 初始化失败：HRESULT 0x{:08x}",
                    initialized.0 as u32
                ));
            }
        }

        let result = (|| {
            let manager: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)
                    .map_err(|error| format!("创建 Windows 激活器失败：{error}"))?;
            manager
                .ActivateApplication(
                    &HSTRING::from(app_user_model_id),
                    &HSTRING::from(arguments),
                    ACTIVATEOPTIONS(0),
                )
                .map_err(|error| format!("激活 Codex 打包应用失败：{error}"))
        })();

        if should_uninitialize {
            CoUninitialize();
        }
        result
    }
}

fn is_primary_target(target: &CdpTarget) -> bool {
    target.target_type.eq_ignore_ascii_case("page")
        && (target.url.eq_ignore_ascii_case("app://-/index.html")
            || (target.title.eq_ignore_ascii_case("Codex")
                && target.url.starts_with("app://-/")
                && !target.url.contains("initialRoute=")))
        && target.web_socket_debugger_url.is_some()
}

fn primary_target_blocking() -> Option<CdpTarget> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(500))
        .build()
        .ok()?;
    let targets = client
        .get(format!("http://127.0.0.1:{DEBUG_PORT}/json"))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Vec<CdpTarget>>()
        .ok()?;
    targets.into_iter().find(is_primary_target)
}

async fn primary_target_async() -> Option<CdpTarget> {
    tauri::async_runtime::spawn_blocking(primary_target_blocking)
        .await
        .ok()
        .flatten()
}

async fn inject_badge(app_version: &str) -> Result<(), String> {
    let target = primary_target_async()
        .await
        .ok_or_else(|| "Codex 页面尚未准备完成".to_owned())?;
    let websocket_url = target
        .web_socket_debugger_url
        .as_deref()
        .ok_or_else(|| "Codex 页面没有可用的调试连接".to_owned())?;
    validate_websocket_url(websocket_url)?;
    let response = evaluate_script(websocket_url, &badge_script(app_version)).await?;
    if response
        .pointer("/result/result/value")
        .and_then(Value::as_bool)
        == Some(true)
    {
        Ok(())
    } else {
        Err("Codex 原生菜单栏尚未准备完成".to_owned())
    }
}

fn validate_websocket_url(websocket_url: &str) -> Result<(), String> {
    let parsed =
        url::Url::parse(websocket_url).map_err(|_| "Codex 返回了无效的调试连接地址".to_owned())?;
    if parsed.scheme() != "ws"
        || parsed.port() != Some(DEBUG_PORT)
        || !parsed
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|address| address.is_loopback())
    {
        return Err("Codex 调试连接不在预期的本机端口，已拒绝注入".to_owned());
    }
    Ok(())
}

async fn evaluate_script(websocket_url: &str, expression: &str) -> Result<Value, String> {
    let (mut socket, _) = tokio::time::timeout(CDP_TIMEOUT, connect_async(websocket_url))
        .await
        .map_err(|_| "连接 Codex 页面超时".to_owned())?
        .map_err(|error| format!("无法连接 Codex 页面：{error}"))?;
    let request = json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "returnByValue": true
        }
    });
    socket
        .send(Message::Text(request.to_string().into()))
        .await
        .map_err(|error| format!("无法发送 Codex 页面注入命令：{error}"))?;

    let response = tokio::time::timeout(CDP_TIMEOUT, async {
        while let Some(message) = socket.next().await {
            let message = message.map_err(|error| error.to_string())?;
            let Message::Text(text) = message else {
                continue;
            };
            let value = serde_json::from_str::<Value>(&text).map_err(|error| error.to_string())?;
            if value.get("id").and_then(Value::as_u64) == Some(1) {
                return Ok(value);
            }
        }
        Err("Codex 页面提前关闭了调试连接".to_owned())
    })
    .await
    .map_err(|_| "等待 Codex 页面注入结果超时".to_owned())??;

    if let Some(exception) = response
        .pointer("/result/exceptionDetails/text")
        .and_then(Value::as_str)
    {
        return Err(format!("Codex 页面拒绝了版本信息注入：{exception}"));
    }
    if let Some(error) = response.pointer("/error/message").and_then(Value::as_str) {
        return Err(format!("Codex 页面注入失败：{error}"));
    }
    Ok(response)
}

fn badge_script(app_version: &str) -> String {
    let label = serde_json::to_string(&format!("Codex Go {app_version}"))
        .expect("badge label should serialize");
    format!(
        r##"(() => {{
  const id = "codex-go-version-badge";
  const label = {label};
  const header = document.querySelector(".app-header-tint");
  const menuBar = Array.from(header?.querySelectorAll?.('[class*="ms-auto"][class*="flex"][class*="items-center"]') || [])
    .find((node) => {{
      const rect = node.getBoundingClientRect();
      return !node.closest(".invisible") && rect.width > 0 && rect.height > 0;
    }});
  let parent = menuBar || null;
  let before = null;
  let nativeButtonClass = "";
  if (menuBar) {{
    const buttons = Array.from(menuBar.querySelectorAll("button"))
      .filter((button) => !button.closest(`#${{id}}`));
    const openLocationButton = buttons.find((button) =>
      /^(打开位置|Open location)$/i.test(button.getAttribute("aria-label") || ""));
    const openLocationGroup = openLocationButton?.closest?.(".inline-flex.self-start.items-stretch.overflow-hidden.rounded-lg");
    const openLocationIndex = buttons.indexOf(openLocationButton);
    nativeButtonClass = openLocationButton
      ? buttons[openLocationIndex + 1]?.className || openLocationButton.className || ""
      : buttons[buttons.length - 1]?.className || "";
    if (openLocationGroup?.parentElement === menuBar) before = openLocationGroup;
    else if (openLocationGroup?.parentElement?.parentElement === menuBar) before = openLocationGroup.parentElement;
    else before = buttons[buttons.length - 1]?.nextSibling || null;
  }} else {{
    const contextSurface = document.querySelector('[data-testid="app-shell-header-context-menu-surface"]');
    const buttons = Array.from(contextSurface?.querySelectorAll?.("button") || [])
      .filter((button) => !button.closest(`#${{id}}`) && button.getBoundingClientRect().width > 0 && button.getBoundingClientRect().height > 0);
    const nativeButton = buttons.find((button) => !button.parentElement?.classList?.contains("inline-flex")) || buttons[0];
    parent = nativeButton?.parentElement || null;
    before = nativeButton || null;
    nativeButtonClass = nativeButton?.className || "";
    if (!parent) {{
      const emptyButtonGroup = Array.from(contextSurface?.querySelectorAll?.("div") || [])
        .find((node) => {{
          const className = String(node.className || "");
          return className.includes("items-center") &&
            (className.includes("justify-end") || className.includes("gap-2"));
        }});
      parent = emptyButtonGroup || null;
      before = emptyButtonGroup?.firstChild || null;
    }}
  }}
  if (!parent) return false;

  let badge = document.getElementById(id);
  if (badge?.dataset.codexGoBadgeVersion !== "2") {{
    badge?.remove();
    badge = null;
  }}
  if (!badge) {{
    badge = document.createElement("div");
    badge.id = id;
    badge.dataset.codexGoBadgeVersion = "2";
    Object.assign(badge.style, {{
      display: "inline-flex", alignItems: "center", height: "100%",
      flex: "0 0 auto", pointerEvents: "none", webkitAppRegion: "no-drag"
    }});
    const trigger = document.createElement("button");
    trigger.type = "button";
    trigger.tabIndex = -1;
    Object.assign(trigger.style, {{ pointerEvents: "none", cursor: "default", letterSpacing: "0" }});
    const dot = document.createElement("span");
    Object.assign(dot.style, {{
      width: "9px", height: "9px", flex: "0 0 auto", borderRadius: "999px",
      display: "inline-block", background: "#34d399", boxShadow: "0 0 8px rgba(52,211,153,.75)"
    }});
    const text = document.createElement("span");
    text.dataset.codexGoVersion = "true";
    trigger.append(dot, text);
    badge.appendChild(trigger);
  }}
  const triggerClasses = String(nativeButtonClass || "").split(/\s+/).filter(Boolean);
  const incompatibleClasses = new Set(["gap-0", "rounded-l-none", "border-l-0", "pl-0.5", "pr-1.5"]);
  const normalizedClasses = triggerClasses.filter((name) => !incompatibleClasses.has(name));
  if (triggerClasses.some((name) => incompatibleClasses.has(name))) {{
    ["gap-1", "rounded-lg", "border-l", "px-2"].forEach((name) => {{
      if (!normalizedClasses.includes(name)) normalizedClasses.push(name);
    }});
  }}
  const fallbackClass = "border-token-border no-drag flex items-center gap-1 border whitespace-nowrap select-none rounded-lg text-token-text-tertiary border-transparent h-token-button-composer px-2 py-0 text-base leading-[18px]";
  badge.querySelector("button").className = normalizedClasses.join(" ") || fallbackClass;
  badge.querySelector("[data-codex-go-version]").textContent = label;
  badge.setAttribute("aria-label", label);
  const trigger = badge.querySelector("button");
  if (trigger) trigger.setAttribute("aria-label", label);
  const safeBefore = before?.parentElement === parent ? before : null;
  if (badge.parentElement !== parent || badge.nextSibling !== safeBefore) {{
    parent.insertBefore(badge, safeBefore);
  }}
  window.__CODEX_GO_LAUNCHED__ = true;
  return true;
}})()"##
    )
}

fn start_injection_task(app: AppHandle, app_version: String) {
    tauri::async_runtime::spawn(async move {
        emit_progress(&app, "connecting", "正在连接 Codex 桌面页面");
        for _ in 0..LAUNCH_ATTEMPTS {
            if primary_target_async().await.is_some() {
                emit_progress(&app, "injecting", "正在注入 Codex Go 版本信息");
                if inject_badge(&app_version).await.is_ok() {
                    start_watchdog(app_version);
                    emit_progress(&app, "complete", "Codex Go 版本信息已注入");
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
}

fn start_watchdog(app_version: String) {
    if WATCHDOG_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(WATCHDOG_INTERVAL).await;
            if primary_target_async().await.is_none() {
                WATCHDOG_RUNNING.store(false, Ordering::Release);
                return;
            }
            let _ = inject_badge(&app_version).await;
        }
    });
}

#[cfg(windows)]
fn desktop_processes(executable: &Path) -> DesktopProcesses {
    use std::mem::size_of;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
            Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
    };

    let expected = normalize_windows_path(executable);
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return DesktopProcesses::default();
        }
        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut matches = Vec::new();
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let name_end = entry
                    .szExeFile
                    .iter()
                    .position(|character| *character == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..name_end]);
                if name.eq_ignore_ascii_case("ChatGPT.exe")
                    || name.eq_ignore_ascii_case("Codex.exe")
                {
                    let process =
                        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                    if !process.is_null() {
                        let mut buffer = vec![0_u16; 32_768];
                        let mut length = buffer.len() as u32;
                        if QueryFullProcessImageNameW(
                            process,
                            PROCESS_NAME_WIN32,
                            buffer.as_mut_ptr(),
                            &mut length,
                        ) != 0
                            && normalize_windows_path(Path::new(&String::from_utf16_lossy(
                                &buffer[..length as usize],
                            ))) == expected
                        {
                            matches.push((entry.th32ProcessID, entry.th32ParentProcessID));
                        }
                        CloseHandle(process);
                    }
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);

        let process_ids = matches.iter().map(|(pid, _)| *pid).collect::<Vec<_>>();
        let process_id_set = process_ids.iter().copied().collect::<HashSet<_>>();
        DesktopProcesses {
            visible_window: has_visible_window(&process_id_set),
            process_ids,
        }
    }
}

#[cfg(not(windows))]
fn desktop_processes(_executable: &Path) -> DesktopProcesses {
    DesktopProcesses::default()
}

#[cfg(windows)]
fn has_visible_window(process_ids: &HashSet<u32>) -> bool {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible},
    };

    struct WindowSearch<'a> {
        process_ids: &'a HashSet<u32>,
        found: bool,
    }

    unsafe extern "system" fn visit_window(window: HWND, parameter: LPARAM) -> i32 {
        let search = &mut *(parameter as *mut WindowSearch<'_>);
        if IsWindowVisible(window) == 0 {
            return 1;
        }
        let mut process_id = 0;
        GetWindowThreadProcessId(window, &mut process_id);
        if search.process_ids.contains(&process_id) {
            search.found = true;
            return 0;
        }
        1
    }

    let mut search = WindowSearch {
        process_ids,
        found: false,
    };
    unsafe {
        EnumWindows(
            Some(visit_window),
            &mut search as *mut WindowSearch<'_> as LPARAM,
        );
    }
    search.found
}

#[cfg(windows)]
fn terminate_background_processes(
    executable: &Path,
    processes: &DesktopProcesses,
) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE},
    };

    for process_id in &processes.process_ids {
        unsafe {
            let process = OpenProcess(PROCESS_TERMINATE, 0, *process_id);
            if !process.is_null() {
                let _ = TerminateProcess(process, 1);
                CloseHandle(process);
            }
        }
    }

    for _ in 0..50 {
        if desktop_processes(executable).process_ids.is_empty() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("无法关闭无窗口的 Codex 后台进程，请在任务管理器中结束 Codex 后重试".to_owned())
}

#[cfg(not(windows))]
fn terminate_background_processes(
    _executable: &Path,
    _processes: &DesktopProcesses,
) -> Result<(), String> {
    Ok(())
}

fn normalize_windows_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn target(url: &str) -> CdpTarget {
        CdpTarget {
            target_type: "page".to_owned(),
            title: "Codex".to_owned(),
            url: url.to_owned(),
            web_socket_debugger_url: Some("ws://127.0.0.1:9230/devtools/page/test".to_owned()),
        }
    }

    #[test]
    fn selects_main_codex_page_only() {
        assert!(is_primary_target(&target("app://-/index.html")));
        assert!(!is_primary_target(&target(
            "app://-/index.html?initialRoute=%2Favatar-overlay"
        )));
    }

    #[test]
    fn badge_contains_product_and_escaped_version() {
        let script = badge_script("1.2.3\"test");
        assert!(script.contains("Codex Go 1.2.3\\\"test"));
        assert!(script.contains("codex-go-version-badge"));
        assert!(script.contains("__CODEX_GO_LAUNCHED__"));
        assert!(script.contains(".app-header-tint"));
        assert!(!script.contains("position: \"fixed\""));
    }

    #[test]
    fn resolves_store_package_executable() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        std::fs::create_dir(&app).unwrap();
        std::fs::write(app.join("ChatGPT.exe"), []).unwrap();
        assert_eq!(
            codex_executable(root.path().to_str().unwrap()),
            Some(app.join("ChatGPT.exe"))
        );
    }

    #[test]
    fn builds_store_application_user_model_id() {
        assert_eq!(
            packaged_app_user_model_id(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_26.727.4816.0_x64__2p2nqsd0c76g0"
            ),
            Some("OpenAI.Codex_2p2nqsd0c76g0!App".to_owned())
        );
    }

    #[test]
    fn hidden_unmanaged_background_process_is_stopped() {
        assert_eq!(
            classify_runtime(true, false, false).state,
            CodexRuntimeState::Stopped
        );
    }

    #[test]
    fn runtime_has_exactly_three_states() {
        assert_eq!(
            classify_runtime(false, false, false).state,
            CodexRuntimeState::Stopped
        );
        assert_eq!(
            classify_runtime(true, true, false).state,
            CodexRuntimeState::Unmanaged
        );
        assert_eq!(
            classify_runtime(true, false, true).state,
            CodexRuntimeState::Managed
        );
    }

    #[cfg(windows)]
    #[test]
    fn hidden_desktop_process_fixture_is_detected_and_terminated() {
        use std::os::windows::process::CommandExt;

        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("ChatGPT.exe");
        let command_prompt = std::env::var_os("COMSPEC").unwrap();
        std::fs::copy(command_prompt, &executable).unwrap();
        let mut child = Command::new(&executable)
            .args(["/c", "ping", "127.0.0.1", "-n", "30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(150));

        let processes = desktop_processes(&executable);
        if processes.process_ids.is_empty() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("hidden ChatGPT fixture was not detected");
        }
        assert!(!processes.visible_window);
        terminate_background_processes(&executable, &processes).unwrap();
        assert!(desktop_processes(&executable).process_ids.is_empty());
        let _ = child.wait();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "launches the installed Windows Codex desktop app"]
    fn packaged_activation_and_badge_injection_smoke_test() {
        tauri::async_runtime::block_on(async {
            let started = std::time::Instant::now();
            let process_id = activate_packaged_app(
                "OpenAI.Codex_2p2nqsd0c76g0!App",
                "--remote-debugging-port=9230 --remote-allow-origins=http://127.0.0.1:9230",
            )
            .await
            .expect("Windows packaged activation should succeed");
            assert!(process_id > 0);
            assert!(started.elapsed() < std::time::Duration::from_secs(3));

            let target = loop {
                if let Some(target) = primary_target_async().await {
                    break target;
                }
                assert!(
                    started.elapsed() < std::time::Duration::from_secs(8),
                    "Codex CDP page did not become ready"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            loop {
                if inject_badge("0.1.1").await.is_ok() {
                    break;
                }
                assert!(
                    started.elapsed() < std::time::Duration::from_secs(8),
                    "Codex Go badge was not injected into the native menu"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let response = evaluate_script(
                target.web_socket_debugger_url.as_deref().unwrap(),
                r#"(() => {
                  const badge = document.getElementById('codex-go-version-badge');
                  const text = badge?.querySelector('[data-codex-go-version]')?.textContent;
                  const parent = badge?.parentElement;
                  return {
                    text,
                    insideHeaderSurface: !!badge?.closest('.app-header-tint, [data-testid="app-shell-header-context-menu-surface"]'),
                    insideNativeMenu: !!parent?.matches('[class*="ms-auto"][class*="flex"][class*="items-center"]') ||
                      !!parent?.closest('[data-testid="app-shell-header-context-menu-surface"]'),
                    position: badge ? getComputedStyle(badge).position : null
                  };
                })()"#,
            )
            .await
            .expect("injected badge should be readable");
            let value = response.pointer("/result/result/value").unwrap();
            assert_eq!(
                value.get("text").and_then(Value::as_str),
                Some("Codex Go 0.1.1")
            );
            assert_eq!(
                value.get("insideHeaderSurface").and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(
                value.get("insideNativeMenu").and_then(Value::as_bool),
                Some(true)
            );
            assert_ne!(value.get("position").and_then(Value::as_str), Some("fixed"));
            eprintln!(
                "packaged activation + badge injection pid={process_id} elapsed={:?}",
                started.elapsed()
            );
        });
    }
}
