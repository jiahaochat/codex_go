use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
use url::Url;
use uuid::Uuid;

use crate::process_guard::ChildJob;

const XRAY_SHA256: &str = "15c2d007954ac53ba69b80ec91242786b3c0b71d52649165b4ca1d5cc96ef8f1";
const MAX_XRAY_BYTES: u64 = 80 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VlessEndpoint {
    pub id: String,
    pub host: String,
    pub port: u16,
    pub encryption: String,
    pub server_name: String,
    pub fingerprint: String,
    pub public_key: String,
    pub short_id: String,
    pub path: String,
    pub mode: Option<String>,
}

pub struct ProxyRuntime {
    child: Child,
    _job: ChildJob,
    pub address: String,
    pub url: String,
    pub username: String,
    pub password: String,
}

impl Drop for ProxyRuntime {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn parse_vless_uri(raw: &str) -> Result<VlessEndpoint, String> {
    if raw.len() > 8 * 1024 {
        return Err("VLESS 链接长度异常".to_owned());
    }
    let url = Url::parse(raw).map_err(|_| "VLESS 链接格式无效".to_owned())?;
    if url.scheme() != "vless" {
        return Err("线路必须使用 vless:// 协议".to_owned());
    }
    let id = url.username();
    Uuid::parse_str(id).map_err(|_| "VLESS 用户 ID 无效".to_owned())?;
    if url.password().is_some() {
        return Err("VLESS 链接包含不支持的密码字段".to_owned());
    }
    let host = url
        .host_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "VLESS 服务器地址缺失".to_owned())?;
    let port = url
        .port()
        .ok_or_else(|| "VLESS 服务器端口缺失".to_owned())?;

    let allowed = [
        "encryption",
        "security",
        "sni",
        "fp",
        "pbk",
        "sid",
        "type",
        "path",
        "mode",
    ];
    let mut query = HashMap::<String, String>::new();
    for (key, value) in url.query_pairs() {
        if !allowed.contains(&key.as_ref()) {
            return Err(format!("VLESS 链接包含不支持的参数: {key}"));
        }
        if query.insert(key.into_owned(), value.into_owned()).is_some() {
            return Err("VLESS 链接包含重复参数".to_owned());
        }
    }

    let required = |name: &str| {
        query
            .get(name)
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .ok_or_else(|| format!("VLESS 链接缺少 {name} 参数"))
    };
    let encryption = required("encryption")?;
    if encryption != "none" {
        return Err("当前仅支持 encryption=none 的 VLESS 线路".to_owned());
    }
    if required("security")? != "reality" {
        return Err("当前仅支持 REALITY 安全层".to_owned());
    }
    if required("type")? != "xhttp" {
        return Err("当前仅支持 XHTTP 传输".to_owned());
    }
    let fingerprint = required("fp")?;
    if ![
        "chrome",
        "firefox",
        "safari",
        "ios",
        "android",
        "edge",
        "360",
        "qq",
        "random",
        "randomized",
    ]
    .contains(&fingerprint.as_str())
    {
        return Err("REALITY 指纹类型不受支持".to_owned());
    }
    let public_key = required("pbk")?;
    if public_key.len() < 40
        || !public_key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("REALITY 公钥格式无效".to_owned());
    }
    let short_id = required("sid")?;
    if short_id.len() > 16
        || short_id.len() % 2 != 0
        || !short_id.chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Err("REALITY short id 格式无效".to_owned());
    }

    let server_name = required("sni")?;
    if server_name.len() > 253 || server_name.chars().any(char::is_whitespace) {
        return Err("REALITY SNI 格式无效".to_owned());
    }
    let path = required("path")?;
    if path.len() > 2048 || path.chars().any(char::is_control) {
        return Err("XHTTP 路径格式无效".to_owned());
    }
    let mode = query.get("mode").cloned();
    if mode
        .as_deref()
        .is_some_and(|value| !["auto", "packet-up", "stream-up", "stream-one"].contains(&value))
    {
        return Err("XHTTP mode 不受支持".to_owned());
    }

    Ok(VlessEndpoint {
        id: id.to_owned(),
        host: host.to_owned(),
        port,
        encryption,
        server_name,
        fingerprint,
        public_key,
        short_id,
        path,
        mode,
    })
}

pub fn xray_available(app: &AppHandle) -> bool {
    find_xray(app).is_some_and(|executable| verify_xray(&executable).is_ok())
}

pub fn start(app: &AppHandle, uri: &str) -> Result<ProxyRuntime, String> {
    let endpoint = parse_vless_uri(uri)?;
    let executable = find_xray(app).ok_or_else(|| "安装包中缺少 Xray 核心".to_owned())?;
    verify_xray(&executable)?;
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|_| "无法分配本机代理端口".to_owned())?;
    let port = listener
        .local_addr()
        .map_err(|_| "无法读取本机代理端口".to_owned())?
        .port();
    drop(listener);

    let user = format!("cg{}", Uuid::new_v4().simple());
    let password = Uuid::new_v4().simple().to_string();
    let config = build_xray_config(&endpoint, port, &user, &password);
    let encoded = serde_json::to_vec(&config).map_err(|_| "无法生成 Xray 配置".to_owned())?;
    test_config(&executable, &encoded)?;

    let mut command = Command::new(&executable);
    command
        .args(["run", "-config", "stdin:"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|_| "无法启动 Xray 核心".to_owned())?;
    let job =
        ChildJob::attach(&mut child).map_err(|_| "无法约束 Xray 子进程生命周期".to_owned())?;
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(&encoded).is_err() || stdin.write_all(b"\n").is_err() {
            let _ = child.kill();
            return Err("无法向 Xray 传递线路配置".to_owned());
        }
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        if child
            .try_wait()
            .map_err(|_| "无法读取 Xray 状态".to_owned())?
            .is_some()
        {
            return Err("Xray 未能启动本机代理".to_owned());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Err("等待 Xray 本机代理超时".to_owned());
        }
        thread::sleep(Duration::from_millis(50));
    }

    Ok(ProxyRuntime {
        child,
        _job: job,
        address: format!("http://127.0.0.1:{port}"),
        url: format!("http://{user}:{password}@127.0.0.1:{port}"),
        username: user,
        password,
    })
}

fn build_xray_config(endpoint: &VlessEndpoint, port: u16, user: &str, password: &str) -> Value {
    let mut xhttp = Map::new();
    xhttp.insert("path".to_owned(), Value::String(endpoint.path.clone()));
    if let Some(mode) = &endpoint.mode {
        xhttp.insert("mode".to_owned(), Value::String(mode.clone()));
    }

    json!({
        "log": { "loglevel": "warning" },
        "inbounds": [{
            "tag": "codex-go-local",
            "listen": "127.0.0.1",
            "port": port,
            "protocol": "http",
            "settings": {
                "accounts": [{ "user": user, "pass": password }]
            }
        }],
        "outbounds": [{
            "tag": "codex-go-vless",
            "protocol": "vless",
            "settings": {
                "vnext": [{
                    "address": endpoint.host,
                    "port": endpoint.port,
                    "users": [{
                        "id": endpoint.id,
                        "encryption": endpoint.encryption
                    }]
                }]
            },
            "streamSettings": {
                "network": "xhttp",
                "security": "reality",
                "realitySettings": {
                    "serverName": endpoint.server_name,
                    "fingerprint": endpoint.fingerprint,
                    "password": endpoint.public_key,
                    "shortId": endpoint.short_id
                },
                "xhttpSettings": Value::Object(xhttp)
            }
        }]
    })
}

fn test_config(executable: &Path, config: &[u8]) -> Result<(), String> {
    let mut command = Command::new(executable);
    command
        .args(["run", "-test", "-config", "stdin:"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|_| "无法校验 Xray 配置".to_owned())?;
    let _job = ChildJob::attach(&mut child).map_err(|_| "无法约束 Xray 校验进程".to_owned())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(config)
            .and_then(|_| stdin.write_all(b"\n"))
            .map_err(|_| "无法校验 Xray 配置".to_owned())?;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child
            .try_wait()
            .map_err(|_| "无法校验 Xray 配置".to_owned())?
        {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Xray 配置校验超时".to_owned());
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    };
    if status.success() {
        Ok(())
    } else {
        Err("Xray 无法加载当前线路配置".to_owned())
    }
}

fn find_xray(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CODEX_GO_XRAY_PATH") {
        let path = PathBuf::from(path);
        if is_file(&path) {
            return Some(path);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("resources/xray/xray.exe"));
        candidates.push(resource_dir.join("xray/xray.exe"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/xray/xray.exe"));
    candidates.into_iter().find(|path| is_file(path))
}

fn is_file(path: &Path) -> bool {
    path.metadata()
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

fn verify_xray(path: &Path) -> Result<(), String> {
    let metadata = path
        .symlink_metadata()
        .map_err(|_| "无法读取 Xray 核心".to_owned())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > MAX_XRAY_BYTES
    {
        return Err("Xray 核心文件异常".to_owned());
    }

    let actual = sha256_file(path)?;
    if actual != XRAY_SHA256 {
        return Err("Xray 核心完整性校验失败".to_owned());
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| "无法读取 Xray 核心".to_owned())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "无法读取 Xray 核心".to_owned())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
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

    const VALID: &str = "vless://11111111-2222-4333-8444-555555555555@example.com:16261?encryption=none&security=reality&sni=cdn.example.com&fp=chrome&pbk=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&sid=0123456789abcdef&type=xhttp&path=unit-test-path#node";

    #[test]
    fn parses_supported_share_uri_without_rewriting_path() {
        let parsed = parse_vless_uri(VALID).unwrap();
        assert_eq!(parsed.host, "example.com");
        assert_eq!(parsed.port, 16261);
        assert_eq!(parsed.path, "unit-test-path");
        assert_eq!(parsed.server_name, "cdn.example.com");
    }

    #[test]
    fn rejects_unknown_or_insecure_parameters() {
        assert!(parse_vless_uri(&VALID.replace("#node", "&extra=%7B%7D#node")).is_err());
        assert!(parse_vless_uri(&VALID.replace("security=reality", "security=none")).is_err());
        assert!(parse_vless_uri(&VALID.replace("type=xhttp", "type=ws")).is_err());
    }

    #[test]
    fn generated_config_keeps_proxy_loopback_and_reality_fields() {
        let endpoint = parse_vless_uri(VALID).unwrap();
        let config = build_xray_config(&endpoint, 19080, "local-user", "local-pass");
        assert_eq!(config["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(
            config["inbounds"][0]["settings"]["accounts"][0]["user"],
            "local-user"
        );
        assert_eq!(config["outbounds"][0]["streamSettings"]["network"], "xhttp");
        assert_eq!(
            config["outbounds"][0]["streamSettings"]["realitySettings"]["password"],
            endpoint.public_key
        );
        assert_eq!(
            config["outbounds"][0]["streamSettings"]["xhttpSettings"]["path"],
            "unit-test-path"
        );
    }

    #[test]
    fn hashes_files_without_loading_them_as_text() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.bin");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
