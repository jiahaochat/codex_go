use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const VERSION_TIMEOUT: Duration = Duration::from_secs(4);
const PLUGIN_LIST_TIMEOUT: Duration = Duration::from_secs(8);
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(25);
const MAX_COMMAND_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_METADATA_FILE_BYTES: u64 = 512 * 1024;
const MAX_WALK_ENTRIES: usize = 20_000;
const MAX_PLUGIN_DEPTH: usize = 10;
const MAX_SKILL_DEPTH: usize = 14;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InventorySource {
    Command,
    Filesystem,
    Config,
    Marketplace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginItem {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
    pub marketplace: Option<String>,
    pub path: Option<String>,
    pub source: InventorySource,
    pub error: Option<String>,
    pub icon: Option<String>,
    pub official: bool,
    pub can_delete: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SkillOrigin {
    Personal,
    Plugin,
    System,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillItem {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub origin: SkillOrigin,
    pub plugin_name: Option<String>,
    pub path: String,
    pub source: InventorySource,
    pub error: Option<String>,
    pub icon: Option<String>,
    pub plugin_icon: Option<String>,
    pub official: bool,
    pub can_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    pub codex_home: String,
    pub codex: CodexStatus,
    pub plugins: Vec<PluginItem>,
    pub skills: Vec<SkillItem>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct ExecutableCandidate {
    path: PathBuf,
    detected_via: &'static str,
}

#[derive(Debug, Clone)]
struct PluginRoot {
    path: PathBuf,
    identity: String,
    name: String,
    icon: Option<String>,
    official: bool,
}

#[derive(Debug)]
struct ProcessOutput {
    success: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[derive(Debug)]
enum ProcessError {
    Io(String),
    TimedOut,
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => formatter.write_str(message),
            Self::TimedOut => formatter.write_str("command timed out"),
        }
    }
}

/// Collects local Codex desktop state and its shared local inventory.
pub fn collect_inventory() -> Inventory {
    let codex_home = resolve_codex_home();
    let (codex, warnings) = detect_codex();

    let filesystem_plugins = scan_plugin_manifests(&codex_home);
    let plugins = deduplicate_plugins(filesystem_plugins);
    let plugin_roots = plugin_roots_from(&plugins);
    let skills = scan_skills(&codex_home, &plugin_roots);

    Inventory {
        codex_home: path_to_string(&codex_home),
        codex,
        plugins,
        skills,
        warnings,
    }
}

/// Detects the official Microsoft Store Codex Windows desktop package.
pub fn detect_codex_status() -> CodexStatus {
    detect_codex().0
}

/// Resolves `CODEX_HOME`, falling back to the current user's `.codex` folder.
pub fn resolve_codex_home() -> PathBuf {
    let explicit = env::var_os("CODEX_HOME");
    let home = BaseDirs::new().map(|directories| directories.home_dir().to_path_buf());
    resolve_codex_home_from(explicit, home)
}

fn resolve_codex_home_from(explicit: Option<OsString>, home: Option<PathBuf>) -> PathBuf {
    if let Some(value) = explicit.filter(|value| !value.is_empty()) {
        return PathBuf::from(value);
    }

    home.unwrap_or_else(|| PathBuf::from(".")).join(".codex")
}

fn detect_codex() -> (CodexStatus, Vec<String>) {
    #[cfg(windows)]
    {
        const QUERY: &str = "Get-AppxPackage -Name OpenAI.Codex -ErrorAction SilentlyContinue | Select-Object @{Name='Version';Expression={$_.Version.ToString()}},InstallLocation,@{Name='Status';Expression={$_.Status.ToString()}} | ConvertTo-Json -Compress";
        match run_process(
            Path::new("powershell.exe"),
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                QUERY,
            ],
            VERSION_TIMEOUT,
        ) {
            Ok(output) if output.success && !output.stdout.trim().is_empty() => {
                match serde_json::from_str::<StorePackage>(&output.stdout) {
                    Ok(package) => {
                        let mut warnings = Vec::new();
                        if package
                            .status
                            .as_deref()
                            .is_some_and(|status| !status.eq_ignore_ascii_case("ok"))
                        {
                            warnings.push("Codex Windows 桌面端的 Microsoft Store 包状态异常，请在 Microsoft Store 中修复或重新安装".to_owned());
                        }
                        return (
                            CodexStatus {
                                installed: true,
                                path: package
                                    .install_location
                                    .map(PathBuf::from)
                                    .as_deref()
                                    .map(path_to_string),
                                version: package.version,
                                source: Some("Microsoft Store".to_owned()),
                            },
                            warnings,
                        );
                    }
                    Err(_) => {
                        return (
                            missing_codex_status(),
                            vec!["无法读取 Codex Windows 桌面端的安装信息".to_owned()],
                        )
                    }
                }
            }
            Ok(_) => return (missing_codex_status(), Vec::new()),
            Err(_) => {
                return (
                    missing_codex_status(),
                    vec!["无法查询 Microsoft Store 应用状态".to_owned()],
                )
            }
        }
    }

    #[cfg(not(windows))]
    (missing_codex_status(), Vec::new())
}

fn missing_codex_status() -> CodexStatus {
    CodexStatus {
        installed: false,
        path: None,
        version: None,
        source: None,
    }
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StorePackage {
    version: Option<String>,
    install_location: Option<String>,
    status: Option<String>,
}

#[allow(dead_code)]
fn executable_candidates() -> Vec<ExecutableCandidate> {
    let mut candidates = Vec::new();

    if let Some(path) = first_codex_on_path(env::var_os("PATH")) {
        candidates.push(ExecutableCandidate {
            path,
            detected_via: "path",
        });
    }

    if let Some(local_app_data) = env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(local_app_data)
            .join("Programs")
            .join("OpenAI")
            .join("Codex")
            .join("bin")
            .join("codex.exe");
        if is_regular_file(&path) {
            candidates.push(ExecutableCandidate {
                path,
                detected_via: "standalone",
            });
        }
    }

    if let Some(app_data) = env::var_os("APPDATA").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(app_data).join("npm").join("codex.cmd");
        if is_regular_file(&path) {
            candidates.push(ExecutableCandidate {
                path,
                detected_via: "npm",
            });
        }
    }

    deduplicate_candidates(candidates)
}

fn first_codex_on_path(path: Option<OsString>) -> Option<PathBuf> {
    const WINDOWS_NAMES: [&str; 4] = ["codex.exe", "codex.cmd", "codex.bat", "codex"];
    const OTHER_NAMES: [&str; 4] = ["codex", "codex.exe", "codex.cmd", "codex.bat"];

    let names = if cfg!(windows) {
        &WINDOWS_NAMES
    } else {
        &OTHER_NAMES
    };

    first_program_on_path(path, names)
}

fn first_program_on_path(path: Option<OsString>, names: &[&str]) -> Option<PathBuf> {
    for directory in env::split_paths(&path?) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        for name in names {
            let candidate = directory.join(name);
            if is_regular_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn deduplicate_candidates(candidates: Vec<ExecutableCandidate>) -> Vec<ExecutableCandidate> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(path_key(&candidate.path)))
        .collect()
}

fn run_process(
    executable: &Path,
    arguments: &[&str],
    timeout: Duration,
) -> Result<ProcessOutput, ProcessError> {
    let arguments = arguments
        .iter()
        .map(|argument| OsString::from(*argument))
        .collect::<Vec<_>>();
    run_process_os(executable, &arguments, timeout)
}

fn run_codex_process(
    executable: &Path,
    arguments: &[&str],
    timeout: Duration,
) -> Result<ProcessOutput, ProcessError> {
    #[cfg(windows)]
    if executable
        .extension()
        .and_then(OsStr::to_str)
        .map(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
        .unwrap_or(false)
    {
        let script = executable
            .parent()
            .map(|parent| {
                parent
                    .join("node_modules")
                    .join("@openai")
                    .join("codex")
                    .join("bin")
                    .join("codex.js")
            })
            .filter(|path| is_regular_file(path))
            .ok_or_else(|| {
                ProcessError::Io("npm Codex JavaScript entry point was not found".to_owned())
            })?;
        let node = first_program_on_path(env::var_os("PATH"), &["node.exe", "node"])
            .ok_or_else(|| ProcessError::Io("node.exe was not found on PATH".to_owned()))?;
        let mut node_arguments = vec![script.into_os_string()];
        node_arguments.extend(arguments.iter().map(|argument| OsString::from(*argument)));
        return run_process_os(&node, &node_arguments, timeout);
    }

    run_process(executable, arguments, timeout)
}

fn run_process_os(
    executable: &Path,
    arguments: &[OsString],
    timeout: Duration,
) -> Result<ProcessOutput, ProcessError> {
    let mut stdout_file = tempfile::tempfile().map_err(process_io_error)?;
    let mut stderr_file = tempfile::tempfile().map_err(process_io_error)?;
    let child_stdout = stdout_file.try_clone().map_err(process_io_error)?;
    let child_stderr = stderr_file.try_clone().map_err(process_io_error)?;

    let mut command = Command::new(executable);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::from(child_stdout))
        .stderr(Stdio::from(child_stderr));

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command.spawn().map_err(process_io_error)?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().map_err(process_io_error)? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProcessError::TimedOut);
            }
            None => thread::sleep(COMMAND_POLL_INTERVAL),
        }
    };

    let stdout = read_process_output(&mut stdout_file)?;
    let stderr = read_process_output(&mut stderr_file)?;
    Ok(ProcessOutput {
        success: status.success(),
        code: status.code(),
        stdout,
        stderr,
    })
}

fn process_io_error(error: std::io::Error) -> ProcessError {
    ProcessError::Io(error.to_string())
}

fn read_process_output(file: &mut File) -> Result<String, ProcessError> {
    file.seek(SeekFrom::Start(0)).map_err(process_io_error)?;
    let mut bytes = Vec::new();
    file.take(MAX_COMMAND_OUTPUT_BYTES)
        .read_to_end(&mut bytes)
        .map_err(process_io_error)?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
}

fn first_nonempty_line(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

fn concise_stderr(stderr: &str) -> String {
    first_nonempty_line(stderr)
        .map(|line| format!(": {line}"))
        .unwrap_or_default()
}

#[allow(dead_code)]
fn list_plugins_with_cli(executable: &Path) -> Result<Vec<PluginItem>, String> {
    let output = run_codex_process(
        executable,
        &["plugin", "list", "--json"],
        PLUGIN_LIST_TIMEOUT,
    )
    .map_err(|error| error.to_string())?;

    if !output.success {
        return Err(format!(
            "command exited with {}{}",
            output
                .code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "an unknown status".to_owned()),
            concise_stderr(&output.stderr)
        ));
    }

    parse_cli_plugins(&output.stdout)
}

fn parse_cli_plugins(json: &str) -> Result<Vec<PluginItem>, String> {
    let value: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let mut plugins = Vec::new();
    if let Some(installed) = value.as_object().and_then(|object| object.get("installed")) {
        collect_cli_plugins(installed, &mut plugins, None, true, 0);
    } else {
        collect_cli_plugins(&value, &mut plugins, None, true, 0);
    }
    Ok(deduplicate_plugins(plugins))
}

fn collect_cli_plugins(
    value: &Value,
    plugins: &mut Vec<PluginItem>,
    fallback_name: Option<&str>,
    plugin_context: bool,
    depth: usize,
) {
    if depth > 10 {
        return;
    }

    match value {
        Value::String(name) if plugin_context => {
            if let Some(name) = nonempty(name) {
                plugins.push(PluginItem {
                    id: name.clone(),
                    name,
                    version: None,
                    description: None,
                    enabled: true,
                    marketplace: None,
                    path: None,
                    source: InventorySource::Command,
                    error: None,
                    icon: None,
                    official: false,
                    can_delete: false,
                });
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_cli_plugins(item, plugins, None, plugin_context, depth + 1);
            }
        }
        Value::Object(object) => {
            let parsed = plugin_from_object(object, fallback_name, plugin_context);
            let parsed_directly = parsed.is_some();
            if let Some(plugin) = parsed {
                plugins.push(plugin);
            }

            for (key, child) in object {
                let normalized_key = normalize_identifier(key);
                if is_plugin_collection_key(&normalized_key) {
                    collect_cli_plugins(child, plugins, None, true, depth + 1);
                } else if is_wrapper_key(&normalized_key) {
                    collect_cli_plugins(child, plugins, None, false, depth + 1);
                } else if !parsed_directly && is_possible_plugin_value(child) {
                    collect_cli_plugins(child, plugins, Some(key), true, depth + 1);
                } else if !parsed_directly
                    && plugin_context
                    && !is_reserved_cli_key(&normalized_key)
                {
                    if let Some(version) = child.as_str().and_then(nonempty) {
                        plugins.push(PluginItem {
                            id: key.clone(),
                            name: key.clone(),
                            version: Some(version),
                            description: None,
                            enabled: true,
                            marketplace: None,
                            path: None,
                            source: InventorySource::Command,
                            error: None,
                            icon: None,
                            official: false,
                            can_delete: false,
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

fn plugin_from_object(
    object: &Map<String, Value>,
    fallback_name: Option<&str>,
    plugin_context: bool,
) -> Option<PluginItem> {
    let explicit_id = string_field(object, &["id", "pluginId", "plugin_id"]);
    let explicit_name = string_field(object, &["name", "pluginName", "plugin_name", "slug"])
        .or_else(|| nested_string_field(object, "plugin", &["name", "id", "slug"]))
        .or_else(|| nested_string_field(object, "manifest", &["name", "id", "slug"]))
        .or_else(|| nested_string_field(object, "metadata", &["name", "id", "slug"]))
        .or_else(|| explicit_id.clone());

    let has_plugin_fields = object.keys().any(|key| {
        matches!(
            normalize_identifier(key).as_str(),
            "version"
                | "description"
                | "path"
                | "location"
                | "directory"
                | "root"
                | "installpath"
                | "installedpath"
                | "enabled"
        )
    });

    let name = explicit_name.or_else(|| {
        (plugin_context && has_plugin_fields)
            .then(|| fallback_name.and_then(nonempty))
            .flatten()
    })?;

    let marketplace = string_field(
        object,
        &[
            "marketplace",
            "marketplaceName",
            "marketplace_name",
            "registry",
            "scope",
        ],
    )
    .or_else(|| {
        nested_string_field(
            object,
            "metadata",
            &[
                "marketplace",
                "marketplaceName",
                "marketplace_name",
                "registry",
                "scope",
            ],
        )
    })
    .or_else(|| {
        nested_string_field(
            object,
            "source",
            &[
                "marketplace",
                "marketplaceName",
                "marketplace_name",
                "registry",
                "scope",
            ],
        )
    });
    let id = plugin_id(&name, marketplace.as_deref(), explicit_id.as_deref());
    let official = is_official_marketplace(marketplace.as_deref());
    let path = string_field(
        object,
        &[
            "path",
            "location",
            "directory",
            "root",
            "installPath",
            "install_path",
            "installedPath",
            "installed_path",
        ],
    )
    .or_else(|| {
        nested_string_field(
            object,
            "source",
            &[
                "path",
                "location",
                "directory",
                "root",
                "installPath",
                "install_path",
                "installedPath",
                "installed_path",
            ],
        )
    });
    let can_delete = path.is_some() && !official;

    Some(PluginItem {
        id,
        name,
        version: string_field(object, &["version"])
            .or_else(|| nested_string_field(object, "plugin", &["version"]))
            .or_else(|| nested_string_field(object, "manifest", &["version"])),
        description: string_field(object, &["description", "summary"])
            .or_else(|| nested_string_field(object, "plugin", &["description", "summary"]))
            .or_else(|| nested_string_field(object, "manifest", &["description", "summary"])),
        enabled: bool_field(object, &["enabled", "active"]).unwrap_or(true),
        marketplace,
        path,
        source: InventorySource::Command,
        error: string_field(object, &["error"]),
        icon: None,
        official,
        can_delete,
    })
}

pub fn delete_plugin(plugin_id: &str) -> Result<(), String> {
    let codex_home = resolve_codex_home();
    let plugins = scan_plugin_manifests(&codex_home);
    let plugin = plugins
        .iter()
        .find(|plugin| plugin.id == plugin_id)
        .ok_or_else(|| "插件已不存在，请刷新后重试".to_owned())?;
    if plugin.official || !plugin.can_delete {
        return Err("OpenAI 官方内置插件不能删除".to_owned());
    }

    let plugin_path = PathBuf::from(
        plugin
            .path
            .as_deref()
            .ok_or_else(|| "该插件没有可删除的本地目录".to_owned())?,
    );
    let cache_root = codex_home.join("plugins").join("cache");
    let target = plugin_container_path(&plugin_path, &cache_root).unwrap_or(plugin_path);
    remove_inventory_directory(&target, &codex_home.join("plugins"), "插件")
}

pub fn delete_skill(skill_id: &str) -> Result<(), String> {
    let codex_home = resolve_codex_home();
    let plugins = scan_plugin_manifests(&codex_home);
    let plugin_roots = plugin_roots_from(&plugins);
    let skills = scan_skills(&codex_home, &plugin_roots);
    let skill = skills
        .iter()
        .find(|skill| skill.id == skill_id)
        .ok_or_else(|| "Skill 已不存在，请刷新后重试".to_owned())?;
    if skill.official || !skill.can_delete {
        return Err("OpenAI 官方内置 Skill 不能删除".to_owned());
    }

    let target = PathBuf::from(&skill.path);
    match skill.origin {
        SkillOrigin::Personal => {
            remove_inventory_directory(&target, &codex_home.join("skills"), "Skill")
        }
        SkillOrigin::Plugin => {
            let plugin_root = plugin_roots
                .iter()
                .find(|plugin| canonical_path_is_within(&target, &plugin.path))
                .ok_or_else(|| "无法确认该 Skill 所属的插件目录".to_owned())?;
            if paths_are_equal(&target, &plugin_root.path) {
                return Err("该 Skill 与插件共用根目录，请从插件列表删除整个插件".to_owned());
            }
            remove_inventory_directory(&target, &plugin_root.path, "Skill")
        }
        SkillOrigin::System | SkillOrigin::Unknown => Err("该 Skill 不能删除".to_owned()),
    }
}

pub fn read_skill_content(skill_id: &str) -> Result<String, String> {
    let codex_home = resolve_codex_home();
    let plugins = scan_plugin_manifests(&codex_home);
    let skills = scan_skills(&codex_home, &plugin_roots_from(&plugins));
    let skill = skills
        .iter()
        .find(|skill| skill.id == skill_id)
        .ok_or_else(|| "Skill 已不存在，请刷新后重试".to_owned())?;
    let path = PathBuf::from(&skill.path).join("SKILL.md");
    read_limited_text(&path, MAX_METADATA_FILE_BYTES)
        .map_err(|_| "无法读取该 Skill 的 SKILL.md".to_owned())
}

fn plugin_container_path(plugin_path: &Path, cache_root: &Path) -> Option<PathBuf> {
    let relative = plugin_path.strip_prefix(cache_root).ok()?;
    let mut components = relative.components();
    let marketplace = components.next()?;
    let plugin_name = components.next()?;
    components.next()?;
    Some(
        cache_root
            .join(marketplace.as_os_str())
            .join(plugin_name.as_os_str()),
    )
}

fn paths_are_equal(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn remove_inventory_directory(
    target: &Path,
    allowed_root: &Path,
    label: &str,
) -> Result<(), String> {
    let target = fs::canonicalize(target).map_err(|_| format!("{label} 目录已不存在"))?;
    let allowed_root =
        fs::canonicalize(allowed_root).map_err(|_| format!("无法确认 {label} 的安全目录边界"))?;
    if target == allowed_root || !target.starts_with(&allowed_root) {
        return Err(format!("拒绝删除安全目录之外的 {label}"));
    }
    let metadata = fs::symlink_metadata(&target).map_err(|_| format!("无法读取 {label} 目录"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("{label} 目标不是可删除的普通目录"));
    }
    fs::remove_dir_all(&target).map_err(|error| format!("删除 {label} 失败：{error}"))
}

fn bool_field(object: &Map<String, Value>, aliases: &[&str]) -> Option<bool> {
    aliases.iter().find_map(|alias| {
        object
            .iter()
            .find(|(key, _)| normalize_identifier(key) == normalize_identifier(alias))
            .and_then(|(_, value)| match value {
                Value::Bool(value) => Some(*value),
                Value::String(value) if value.eq_ignore_ascii_case("true") => Some(true),
                Value::String(value) if value.eq_ignore_ascii_case("false") => Some(false),
                _ => None,
            })
    })
}

fn plugin_id(name: &str, marketplace: Option<&str>, explicit_id: Option<&str>) -> String {
    explicit_id
        .and_then(nonempty)
        .unwrap_or_else(|| match marketplace.and_then(nonempty) {
            Some(marketplace) => format!("{name}@{marketplace}"),
            None => name.to_owned(),
        })
}

fn string_field(object: &Map<String, Value>, aliases: &[&str]) -> Option<String> {
    aliases.iter().find_map(|alias| {
        object
            .iter()
            .find(|(key, _)| normalize_identifier(key) == normalize_identifier(alias))
            .and_then(|(_, value)| value.as_str())
            .and_then(nonempty)
    })
}

fn nested_string_field(
    object: &Map<String, Value>,
    parent: &str,
    aliases: &[&str],
) -> Option<String> {
    object
        .iter()
        .find(|(key, _)| normalize_identifier(key) == normalize_identifier(parent))
        .and_then(|(_, value)| value.as_object())
        .and_then(|nested| string_field(nested, aliases))
}

fn normalize_identifier(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character != '_' && *character != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_plugin_collection_key(key: &str) -> bool {
    matches!(
        key,
        "plugins"
            | "pluginlist"
            | "installedplugins"
            | "items"
            | "entries"
            | "results"
            | "installed"
    )
}

fn is_wrapper_key(key: &str) -> bool {
    matches!(key, "data" | "result" | "response" | "payload")
}

fn is_reserved_cli_key(key: &str) -> bool {
    matches!(
        key,
        "status"
            | "message"
            | "count"
            | "total"
            | "version"
            | "schema"
            | "meta"
            | "metadata"
            | "error"
    )
}

fn is_possible_plugin_value(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.keys().any(|key| {
            matches!(
                normalize_identifier(key).as_str(),
                "name"
                    | "id"
                    | "pluginname"
                    | "slug"
                    | "version"
                    | "description"
                    | "path"
                    | "location"
                    | "directory"
                    | "root"
                    | "installpath"
                    | "installedpath"
                    | "manifest"
            )
        }),
        Value::Array(_) => true,
        _ => false,
    }
}

fn scan_plugin_manifests(codex_home: &Path) -> Vec<PluginItem> {
    let mut manifests = HashSet::new();
    let mut plugins = Vec::new();

    for root in plugin_scan_roots(codex_home) {
        for manifest in find_named_files(&root, OsStr::new("plugin.json"), MAX_PLUGIN_DEPTH) {
            if manifest.parent().and_then(Path::file_name) != Some(OsStr::new(".codex-plugin")) {
                continue;
            }
            if !manifests.insert(path_key(&manifest)) {
                continue;
            }
            if let Some(plugin) = read_plugin_manifest(&manifest) {
                plugins.push(plugin);
            }
        }
    }

    deduplicate_plugins(plugins)
}

fn read_plugin_manifest(manifest_path: &Path) -> Option<PluginItem> {
    let contents = read_limited_text(manifest_path, MAX_METADATA_FILE_BYTES).ok()?;
    let value: Value = serde_json::from_str(&contents).ok()?;
    let object = value.as_object()?;
    let plugin_root = manifest_path.parent()?.parent()?;
    let fallback_name = plugin_root.file_name()?.to_string_lossy();

    let explicit_id = string_field(object, &["id"]);
    let name = string_field(object, &["name", "pluginName", "plugin_name", "slug"])
        .or_else(|| nested_string_field(object, "plugin", &["name", "id", "slug"]))
        .or_else(|| nested_string_field(object, "metadata", &["name", "id", "slug"]))
        .or_else(|| explicit_id.clone())
        .or_else(|| nonempty(&fallback_name))?;

    let marketplace = string_field(object, &["marketplace", "registry", "scope"])
        .or_else(|| nested_string_field(object, "plugin", &["marketplace", "registry", "scope"]))
        .or_else(|| nested_string_field(object, "metadata", &["marketplace", "registry", "scope"]))
        .or_else(|| infer_marketplace(plugin_root));
    let id = plugin_id(&name, marketplace.as_deref(), explicit_id.as_deref());
    let official = is_official_marketplace(marketplace.as_deref());
    let icon_path = nested_string_field(
        object,
        "interface",
        &[
            "composerIcon",
            "composer_icon",
            "logo",
            "logoDark",
            "logo_dark",
        ],
    );
    let icon = icon_path
        .as_deref()
        .and_then(|path| load_icon_data(plugin_root, plugin_root, path));

    Some(PluginItem {
        id,
        name,
        version: string_field(object, &["version"])
            .or_else(|| nested_string_field(object, "plugin", &["version"]))
            .or_else(|| nested_string_field(object, "metadata", &["version"])),
        description: string_field(object, &["description", "summary"])
            .or_else(|| nested_string_field(object, "plugin", &["description", "summary"]))
            .or_else(|| nested_string_field(object, "metadata", &["description", "summary"])),
        enabled: bool_field(object, &["enabled", "active"]).unwrap_or(true),
        marketplace,
        path: Some(path_to_string(plugin_root)),
        source: InventorySource::Filesystem,
        error: None,
        icon,
        official,
        can_delete: !official,
    })
}

fn is_official_marketplace(marketplace: Option<&str>) -> bool {
    marketplace.is_some_and(|value| {
        matches!(
            normalize_identifier(value).as_str(),
            "openaibundled" | "openaiprimaryruntime"
        )
    })
}

fn infer_marketplace(plugin_root: &Path) -> Option<String> {
    let components = plugin_root.components().collect::<Vec<_>>();
    let cache_index = components
        .iter()
        .rposition(|component| component.as_os_str() == OsStr::new("cache"))?;
    // Expected cache layout: cache/<marketplace>/<plugin>/<version>.
    if components.len().saturating_sub(cache_index + 1) < 3 {
        return None;
    }
    nonempty(&components[cache_index + 1].as_os_str().to_string_lossy())
}

fn plugin_scan_roots(codex_home: &Path) -> Vec<PathBuf> {
    vec![
        codex_home.join("plugins").join("cache"),
        codex_home.join("plugins"),
        codex_home.join("state").join("plugins"),
    ]
}

fn merge_plugins(cli: Vec<PluginItem>, filesystem: Vec<PluginItem>) -> Vec<PluginItem> {
    let mut merged = HashMap::<String, PluginItem>::new();
    for plugin in filesystem.into_iter().chain(cli) {
        let key = plugin_key(&plugin);
        match merged.get_mut(&key) {
            Some(existing) => merge_plugin(existing, plugin),
            None => {
                merged.insert(key, plugin);
            }
        }
    }
    sort_plugins(merged.into_values().collect())
}

fn deduplicate_plugins(plugins: Vec<PluginItem>) -> Vec<PluginItem> {
    merge_plugins(Vec::new(), plugins)
}

fn merge_plugin(existing: &mut PluginItem, incoming: PluginItem) {
    let incoming_is_cli = incoming.source == InventorySource::Command;
    if existing.icon.is_none() {
        existing.icon = incoming.icon.clone();
    }
    existing.official |= incoming.official;
    existing.can_delete = (existing.can_delete || incoming.can_delete) && !existing.official;
    if incoming_is_cli || existing.version.is_none() {
        existing.version = incoming.version.or_else(|| existing.version.take());
    }
    if incoming_is_cli || existing.description.is_none() {
        existing.description = incoming.description.or_else(|| existing.description.take());
    }
    if incoming_is_cli || existing.path.is_none() {
        existing.path = incoming.path.or_else(|| existing.path.take());
    }
    if incoming_is_cli {
        existing.id = incoming.id;
        existing.enabled = incoming.enabled;
        existing.marketplace = incoming.marketplace.or_else(|| existing.marketplace.take());
        existing.error = incoming.error;
        existing.source = incoming.source;
        existing.name = incoming.name;
    }
}

fn plugin_key(plugin: &PluginItem) -> String {
    canonical_plugin_identity(plugin).to_lowercase()
}

fn canonical_plugin_identity(plugin: &PluginItem) -> String {
    let id = nonempty(&plugin.id).unwrap_or_else(|| plugin.name.trim().to_owned());
    if id.contains('@') {
        return id;
    }

    match plugin.marketplace.as_deref().and_then(nonempty) {
        Some(marketplace) => format!("{id}@{marketplace}"),
        None => id,
    }
}

fn plugin_roots_from(plugins: &[PluginItem]) -> Vec<PluginRoot> {
    let mut seen = HashSet::new();
    plugins
        .iter()
        .filter_map(|plugin| {
            let path = PathBuf::from(plugin.path.as_ref()?);
            let identity = canonical_plugin_identity(plugin);
            let key = format!("{}\0{}", path_key(&path), identity.to_lowercase());
            seen.insert(key).then(|| PluginRoot {
                path,
                identity,
                name: plugin.name.clone(),
                icon: plugin.icon.clone(),
                official: plugin.official,
            })
        })
        .collect()
}

fn sort_plugins(mut plugins: Vec<PluginItem>) -> Vec<PluginItem> {
    plugins.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.path.cmp(&right.path))
    });
    plugins
}

fn scan_skills(codex_home: &Path, plugin_roots: &[PluginRoot]) -> Vec<SkillItem> {
    let mut skills = Vec::new();
    let mut seen_paths = HashSet::new();

    let user_skill_root = codex_home.join("skills");
    for skill_file in find_named_files(&user_skill_root, OsStr::new("SKILL.md"), MAX_SKILL_DEPTH) {
        if seen_paths.insert(path_key(&skill_file)) {
            let origin = local_skill_origin(&skill_file, &user_skill_root);
            if let Some(skill) = read_skill(&skill_file, origin, None) {
                skills.push(skill);
            }
        }
    }

    for plugin in plugin_roots {
        for skill_file in plugin_skill_files(&plugin.path) {
            if !seen_paths.insert(path_key(&skill_file)) {
                continue;
            }
            if let Some(skill) = read_skill(&skill_file, SkillOrigin::Plugin, Some(plugin)) {
                skills.push(skill);
            }
        }
    }

    deduplicate_skills(skills)
}

fn local_skill_origin(skill_file: &Path, skills_root: &Path) -> SkillOrigin {
    match skill_file
        .strip_prefix(skills_root)
        .ok()
        .and_then(|relative| relative.components().next())
    {
        Some(component) if component.as_os_str() == OsStr::new(".system") => SkillOrigin::System,
        Some(_) => SkillOrigin::Personal,
        None => SkillOrigin::Unknown,
    }
}

fn plugin_skill_files(plugin_root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let manifest_path = plugin_root.join(".codex-plugin").join("plugin.json");
    let declared = declared_skill_paths(&manifest_path);

    for relative in declared {
        let root = plugin_root.join(relative);
        match fs::symlink_metadata(&root) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                if root.file_name() == Some(OsStr::new("SKILL.md"))
                    && canonical_path_is_within(&root, plugin_root)
                {
                    files.push(root);
                }
            }
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                files.extend(find_named_files(
                    &root,
                    OsStr::new("SKILL.md"),
                    MAX_SKILL_DEPTH,
                ));
            }
            _ => {}
        }
    }

    files.sort_by_key(|path| path_key(path));
    files.dedup_by(|left, right| path_key(left) == path_key(right));
    files
}

fn declared_skill_paths(manifest_path: &Path) -> Vec<PathBuf> {
    let manifest = read_limited_text(manifest_path, MAX_METADATA_FILE_BYTES)
        .ok()
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok());
    let Some(object) = manifest.as_ref().and_then(Value::as_object) else {
        return vec![PathBuf::from("skills")];
    };

    let Some(value) = object
        .iter()
        .find(|(key, _)| normalize_identifier(key) == "skills")
        .map(|(_, value)| value)
    else {
        return vec![PathBuf::from("skills")];
    };

    match value {
        Value::String(path) => safe_manifest_relative_path(path).into_iter().collect(),
        Value::Array(paths) => paths
            .iter()
            .filter_map(Value::as_str)
            .filter_map(safe_manifest_relative_path)
            .collect(),
        _ => Vec::new(),
    }
}

fn safe_manifest_relative_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty()
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.as_bytes().get(1) == Some(&b':')
    {
        return None;
    }

    let path = Path::new(value);
    if path.is_absolute() {
        return None;
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(component) => normalized.push(component),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn canonical_path_is_within(path: &Path, root: &Path) -> bool {
    let Ok(path) = fs::canonicalize(path) else {
        return false;
    };
    let Ok(root) = fs::canonicalize(root) else {
        return false;
    };
    path.starts_with(root)
}

fn read_skill(path: &Path, origin: SkillOrigin, plugin: Option<&PluginRoot>) -> Option<SkillItem> {
    let contents = read_limited_text(path, MAX_METADATA_FILE_BYTES).ok()?;
    let (frontmatter_name, description) = parse_skill_frontmatter(&contents);
    let skill_directory = path.parent()?;
    let fallback_name = skill_directory.file_name()?.to_string_lossy();
    let name = frontmatter_name.or_else(|| nonempty(&fallback_name))?;
    let id = match plugin {
        Some(plugin) => format!("{}/{name}", plugin.identity),
        None => name.clone(),
    };
    let official = origin == SkillOrigin::System || plugin.is_some_and(|plugin| plugin.official);
    let icon_root = plugin.map_or(skill_directory, |plugin| plugin.path.as_path());
    let icon = read_skill_icon(skill_directory, icon_root);
    let plugin_icon = plugin.and_then(|plugin| plugin.icon.clone());
    let can_delete = matches!(origin, SkillOrigin::Personal)
        || (matches!(origin, SkillOrigin::Plugin) && !official);

    Some(SkillItem {
        id,
        name,
        description,
        origin,
        plugin_name: plugin.map(|plugin| plugin.name.clone()),
        path: path_to_string(skill_directory),
        source: InventorySource::Filesystem,
        error: None,
        icon,
        plugin_icon,
        official,
        can_delete,
    })
}

fn read_skill_icon(skill_directory: &Path, allowed_root: &Path) -> Option<String> {
    let metadata_path = skill_directory.join("agents").join("openai.yaml");
    let contents = read_limited_text(&metadata_path, MAX_METADATA_FILE_BYTES).ok()?;
    let relative_path = yaml_scalar_field(&contents, "icon_small")
        .or_else(|| yaml_scalar_field(&contents, "icon_large"))?;
    load_icon_data(skill_directory, allowed_root, &relative_path)
}

fn yaml_scalar_field(contents: &str, field: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let trimmed = line.trim();
        let (key, value) = trimmed.split_once(':')?;
        (normalize_identifier(key) == normalize_identifier(field))
            .then(|| parse_yaml_scalar(value))
            .flatten()
    })
}

fn load_icon_data(base: &Path, allowed_root: &Path, relative_path: &str) -> Option<String> {
    let relative = Path::new(relative_path.trim());
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        return None;
    }

    let path = fs::canonicalize(base.join(relative)).ok()?;
    let root = fs::canonicalize(allowed_root).ok()?;
    if !path.starts_with(&root) {
        return None;
    }

    let mime = match path
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("ico") => "image/x-icon",
        _ => return None,
    };
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_METADATA_FILE_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    Some(format!(
        "data:{mime};base64,{}",
        BASE64_STANDARD.encode(bytes)
    ))
}

fn parse_skill_frontmatter(contents: &str) -> (Option<String>, Option<String>) {
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    let lines = contents.lines().collect::<Vec<_>>();
    if !matches!(lines.first(), Some(line) if line.trim() == "---") {
        return (None, None);
    }

    let end = match lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| line.trim() == "---")
        .map(|(index, _)| index)
    {
        Some(index) => index,
        None => return (None, None),
    };

    let mut name = None;
    let mut description = None;
    let mut index = 1;
    while index < end {
        let line = lines[index];
        if line.starts_with(char::is_whitespace) {
            index += 1;
            continue;
        }

        let Some((key, raw_value)) = line.split_once(':') else {
            index += 1;
            continue;
        };
        let key = key.trim();
        if key != "name" && key != "description" {
            index += 1;
            continue;
        }

        let raw_value = raw_value.trim();
        let (value, next_index) = if raw_value.starts_with('|') || raw_value.starts_with('>') {
            parse_yaml_block(&lines, index + 1, end, raw_value.starts_with('>'))
        } else {
            (parse_yaml_scalar(raw_value), index + 1)
        };

        if key == "name" {
            name = value;
        } else {
            description = value;
        }
        index = next_index;
    }

    (name, description)
}

fn parse_yaml_block(
    lines: &[&str],
    start: usize,
    end: usize,
    folded: bool,
) -> (Option<String>, usize) {
    let mut index = start;
    let mut values = Vec::new();
    while index < end {
        let line = lines[index];
        if !line.trim().is_empty() && !line.starts_with(char::is_whitespace) {
            break;
        }
        values.push(line.trim().to_owned());
        index += 1;
    }

    let separator = if folded { " " } else { "\n" };
    let value = values.join(separator).trim().to_owned();
    ((!value.is_empty()).then_some(value), index)
}

fn parse_yaml_scalar(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('#') || value == "null" || value == "~" {
        return None;
    }

    if value.starts_with('"') && value.ends_with('"') {
        return serde_json::from_str::<String>(value)
            .ok()
            .and_then(|value| nonempty(&value));
    }

    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return nonempty(&value[1..value.len() - 1].replace("''", "'"));
    }

    nonempty(value)
}

fn deduplicate_skills(skills: Vec<SkillItem>) -> Vec<SkillItem> {
    let mut merged = HashMap::<String, SkillItem>::new();
    for skill in skills {
        let key = skill.id.trim().to_lowercase();
        match merged.get_mut(&key) {
            Some(existing) => {
                let incoming_is_local =
                    matches!(skill.origin, SkillOrigin::Personal | SkillOrigin::System);
                let existing_is_local =
                    matches!(existing.origin, SkillOrigin::Personal | SkillOrigin::System);
                if incoming_is_local && !existing_is_local {
                    let mut replacement = skill;
                    if replacement.description.is_none() {
                        replacement.description = existing.description.take();
                    }
                    *existing = replacement;
                } else if existing.description.is_none() {
                    existing.description = skill.description;
                }
            }
            None => {
                merged.insert(key, skill);
            }
        }
    }

    let mut skills = merged.into_values().collect::<Vec<_>>();
    skills.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.path.cmp(&right.path))
    });
    skills
}

fn find_named_files(root: &Path, file_name: &OsStr, max_depth: usize) -> Vec<PathBuf> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        _ => return Vec::new(),
    }

    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut visited = HashSet::new();
    let mut files = Vec::new();
    let mut inspected = 0usize;

    while let Some((directory, depth)) = stack.pop() {
        if inspected >= MAX_WALK_ENTRIES {
            break;
        }

        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            _ => continue,
        }

        let canonical_directory = match fs::canonicalize(&directory) {
            Ok(path) if path.starts_with(&canonical_root) => path,
            _ => continue,
        };
        if !visited.insert(path_key(&canonical_directory)) {
            continue;
        }

        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            inspected += 1;
            if inspected > MAX_WALK_ENTRIES {
                break;
            }
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) if !metadata.file_type().is_symlink() => metadata,
                _ => continue,
            };

            if metadata.is_file() && entry.file_name() == file_name {
                files.push(path);
            } else if metadata.is_dir() && depth < max_depth {
                stack.push((path, depth + 1));
            }
        }
    }

    files.sort_by_key(|left| path_key(left));
    files
}

fn read_limited_text(path: &Path, max_bytes: u64) -> std::io::Result<String> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "metadata file is too large",
        ));
    }
    String::from_utf8(bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn is_regular_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

fn path_key(path: &Path) -> String {
    let path = path.to_string_lossy();
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.into_owned()
    }
}

fn path_to_string(path: &Path) -> String {
    let path = path.to_string_lossy();
    if cfg!(windows) {
        path.replace('/', "\\")
    } else {
        path.into_owned()
    }
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn resolves_explicit_codex_home_and_home_fallback() {
        assert_eq!(
            resolve_codex_home_from(
                Some(OsString::from("C:/custom-codex")),
                Some(PathBuf::from("C:/Users/example")),
            ),
            PathBuf::from("C:/custom-codex")
        );
        assert_eq!(
            resolve_codex_home_from(None, Some(PathBuf::from("C:/Users/example"))),
            PathBuf::from("C:/Users/example").join(".codex")
        );
    }

    #[test]
    fn finds_codex_executable_in_the_first_matching_path_directory() {
        let temporary = tempdir().unwrap();
        let first = temporary.path().join("first");
        let second = temporary.path().join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let executable_name = if cfg!(windows) { "codex.exe" } else { "codex" };
        fs::write(second.join(executable_name), b"test").unwrap();
        let path = env::join_paths([&first, &second]).unwrap();

        assert_eq!(
            first_codex_on_path(Some(path)),
            Some(second.join(executable_name))
        );
    }

    #[test]
    fn parses_cli_plugin_arrays_wrappers_and_name_maps() {
        let plugins = parse_cli_plugins(
            r#"{
                "data": {
                    "plugins": [
                        {"name": "Alpha", "version": "1.2.3"},
                        {"pluginName": "beta", "installPath": "C:/beta"},
                        {"delta": "4.0"}
                    ]
                },
                "gamma": {"version": "3.0", "description": "Mapped plugin"}
            }"#,
        )
        .unwrap();

        assert_eq!(
            plugins
                .iter()
                .map(|plugin| plugin.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "beta", "delta", "gamma"]
        );
        assert_eq!(plugins[0].version.as_deref(), Some("1.2.3"));
        assert_eq!(plugins[1].path.as_deref(), Some("C:/beta"));
        assert_eq!(plugins[2].version.as_deref(), Some("4.0"));
    }

    #[test]
    fn parses_current_codex_installed_contract_and_ignores_available_plugins() {
        let plugins = parse_cli_plugins(
            r#"{
                "installed": [{
                    "pluginId": "docs@personal",
                    "name": "docs",
                    "marketplaceName": "personal",
                    "version": "1.4.2",
                    "source": {
                        "source": "marketplace",
                        "path": "C:/plugins/docs/1.4.2"
                    },
                    "enabled": false
                }],
                "available": [{
                    "pluginId": "not-installed@openai",
                    "name": "not-installed",
                    "marketplaceName": "openai",
                    "version": "9.9.9",
                    "enabled": true
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "docs@personal");
        assert_eq!(plugins[0].marketplace.as_deref(), Some("personal"));
        assert_eq!(plugins[0].path.as_deref(), Some("C:/plugins/docs/1.4.2"));
        assert!(!plugins[0].enabled);
    }

    #[test]
    fn scans_plugin_manifests_and_bundled_skills() {
        let temporary = tempdir().unwrap();
        let codex_home = temporary.path().join(".codex");
        let plugin_root = codex_home.join("plugins/cache/acme/Acme/1.0.0");
        let manifest_directory = plugin_root.join(".codex-plugin");
        let skill_directory = plugin_root.join("skills/official/reviewer");
        let internal_skill_directory = plugin_root.join("bridge/internal/reviewer");
        fs::create_dir_all(&manifest_directory).unwrap();
        fs::create_dir_all(&skill_directory).unwrap();
        fs::create_dir_all(&internal_skill_directory).unwrap();
        fs::write(
            manifest_directory.join("plugin.json"),
            r#"{"name":"Acme","version":"1.0.0","description":"Test plugin","skills":"./skills/official"}"#,
        )
        .unwrap();
        fs::write(
            skill_directory.join("SKILL.md"),
            "---\nname: reviewer\ndescription: 'Reviews code'\n---\n",
        )
        .unwrap();
        fs::write(
            internal_skill_directory.join("SKILL.md"),
            "---\nname: reviewer\ndescription: 'Internal duplicate'\n---\n",
        )
        .unwrap();

        let plugins = scan_plugin_manifests(&codex_home);
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "Acme");

        let roots = plugin_roots_from(&plugins);
        let skills = scan_skills(&codex_home, &roots);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "reviewer");
        assert_eq!(skills[0].id, "Acme@acme/reviewer");
        assert_eq!(skills[0].plugin_name.as_deref(), Some("Acme"));
        assert_eq!(skills[0].description.as_deref(), Some("Reviews code"));
        assert_eq!(skills[0].path, path_to_string(&skill_directory));
    }

    #[test]
    fn keeps_same_named_plugins_and_skills_from_different_marketplaces() {
        let temporary = tempdir().unwrap();
        let codex_home = temporary.path().join(".codex");
        let mut plugins = Vec::new();

        for marketplace in ["personal", "team"] {
            let plugin_root = codex_home
                .join("plugins/cache")
                .join(marketplace)
                .join("docs/1.0.0");
            let manifest_directory = plugin_root.join(".codex-plugin");
            let skill_directory = plugin_root.join("skills/search");
            fs::create_dir_all(&manifest_directory).unwrap();
            fs::create_dir_all(&skill_directory).unwrap();
            fs::write(
                manifest_directory.join("plugin.json"),
                format!(r#"{{"name":"docs","marketplace":"{marketplace}","skills":["skills"]}}"#),
            )
            .unwrap();
            fs::write(skill_directory.join("SKILL.md"), "---\nname: search\n---\n").unwrap();

            plugins.push(read_plugin_manifest(&manifest_directory.join("plugin.json")).unwrap());
        }

        let plugins = deduplicate_plugins(plugins);
        assert_eq!(plugins.len(), 2);
        let skills = scan_skills(&codex_home, &plugin_roots_from(&plugins));
        assert_eq!(skills.len(), 2);
        assert_eq!(
            skills
                .iter()
                .map(|skill| skill.id.as_str())
                .collect::<Vec<_>>(),
            vec!["docs@personal/search", "docs@team/search"]
        );
    }

    #[test]
    fn recognizes_official_inventory_and_loads_declared_icons() {
        let temporary = tempdir().unwrap();
        let codex_home = temporary.path().join(".codex");
        let plugin_root = codex_home.join("plugins/cache/openai-bundled/docs/1.0.0");
        let manifest_directory = plugin_root.join(".codex-plugin");
        let skill_directory = plugin_root.join("skills/docs");
        fs::create_dir_all(manifest_directory.as_path()).unwrap();
        fs::create_dir_all(plugin_root.join("assets")).unwrap();
        fs::create_dir_all(skill_directory.join("agents")).unwrap();
        fs::create_dir_all(skill_directory.join("assets")).unwrap();
        fs::write(plugin_root.join("assets/plugin.svg"), "<svg></svg>").unwrap();
        fs::write(skill_directory.join("assets/skill.png"), b"png").unwrap();
        fs::write(
            manifest_directory.join("plugin.json"),
            r#"{"name":"docs","skills":"./skills","interface":{"composerIcon":"./assets/plugin.svg"}}"#,
        )
        .unwrap();
        fs::write(skill_directory.join("SKILL.md"), "---\nname: docs\n---\n").unwrap();
        fs::write(
            skill_directory.join("agents/openai.yaml"),
            "interface:\n  icon_small: \"./assets/skill.png\"\n",
        )
        .unwrap();

        let plugins = scan_plugin_manifests(&codex_home);
        assert_eq!(plugins.len(), 1);
        assert!(plugins[0].official);
        assert!(!plugins[0].can_delete);
        assert!(plugins[0]
            .icon
            .as_deref()
            .is_some_and(|icon| icon.starts_with("data:image/svg+xml;base64,")));

        let skills = scan_skills(&codex_home, &plugin_roots_from(&plugins));
        assert_eq!(skills.len(), 1);
        assert!(skills[0].official);
        assert!(!skills[0].can_delete);
        assert!(skills[0]
            .icon
            .as_deref()
            .is_some_and(|icon| icon.starts_with("data:image/png;base64,")));
        assert!(skills[0]
            .plugin_icon
            .as_deref()
            .is_some_and(|icon| icon.starts_with("data:image/svg+xml;base64,")));
    }

    #[test]
    fn deletion_stays_within_the_allowed_inventory_root() {
        let temporary = tempdir().unwrap();
        let allowed = temporary.path().join("plugins");
        let target = allowed.join("marketplace/plugin");
        let outside = temporary.path().join("outside");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&outside).unwrap();

        assert!(remove_inventory_directory(&outside, &allowed, "插件").is_err());
        assert!(outside.exists());
        remove_inventory_directory(&target, &allowed, "插件").unwrap();
        assert!(!target.exists());
    }

    #[test]
    fn rejects_unsafe_manifest_skill_paths() {
        for path in ["../outside", "/outside", r"C:\\outside", r"\\server\\share"] {
            assert_eq!(safe_manifest_relative_path(path), None);
        }
        assert_eq!(
            safe_manifest_relative_path("./skills/reviewer"),
            Some(PathBuf::from("skills/reviewer"))
        );
    }

    #[test]
    fn parses_simple_and_folded_skill_frontmatter() {
        let (name, description) = parse_skill_frontmatter(
            "---\r\nname: \"release-helper\"\r\ndescription: >-\r\n  Prepares releases\r\n  and notes.\r\n---\r\n",
        );
        assert_eq!(name.as_deref(), Some("release-helper"));
        assert_eq!(description.as_deref(), Some("Prepares releases and notes."));
    }

    #[test]
    fn public_items_serialize_to_frontend_field_names() {
        let skill = SkillItem {
            id: "demo".to_owned(),
            name: "demo".to_owned(),
            description: None,
            origin: SkillOrigin::Personal,
            plugin_name: None,
            path: "C:/Users/example/.codex/skills/demo".to_owned(),
            source: InventorySource::Filesystem,
            error: None,
            icon: None,
            plugin_icon: None,
            official: false,
            can_delete: true,
        };
        let value = serde_json::to_value(skill).unwrap();

        assert_eq!(value["pluginName"], Value::Null);
        assert_eq!(value["origin"], "personal");
        assert_eq!(value["source"], "filesystem");
        assert_eq!(value["canDelete"], true);
    }

    #[cfg(unix)]
    #[test]
    fn recursive_scan_does_not_follow_symbolic_links() {
        use std::os::unix::fs::symlink;

        let temporary = tempdir().unwrap();
        let skills_root = temporary.path().join("skills");
        let outside = temporary.path().join("outside/hidden");
        fs::create_dir_all(&skills_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("SKILL.md"), "---\nname: hidden\n---\n").unwrap();
        symlink(outside, skills_root.join("linked")).unwrap();

        assert!(find_named_files(&skills_root, OsStr::new("SKILL.md"), 5).is_empty());
    }
}
