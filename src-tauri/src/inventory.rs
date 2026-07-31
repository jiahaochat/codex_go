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

/// Collects all local Codex information without entering an async runtime.
///
/// The function performs bounded subprocess calls and is intended to be called
/// through `spawn_blocking` from an async Tauri command.
pub fn collect_inventory() -> Inventory {
    let codex_home = resolve_codex_home();
    let (codex, executable, mut warnings) = detect_codex();

    let filesystem_plugins = scan_plugin_manifests(&codex_home);

    let cli_plugins = executable
        .as_deref()
        .and_then(|path| match list_plugins_with_cli(path) {
            Ok(plugins) => Some(plugins),
            Err(error) => {
                warnings.push(format!("Unable to query Codex plugins: {error}"));
                None
            }
        })
        .unwrap_or_default();

    let plugins = merge_plugins(cli_plugins, filesystem_plugins);
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

/// Detects only the Codex executable and version.
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

fn detect_codex() -> (CodexStatus, Option<PathBuf>, Vec<String>) {
    let candidates = executable_candidates();
    if candidates.is_empty() {
        return (
            CodexStatus {
                installed: false,
                path: None,
                version: None,
                source: None,
            },
            None,
            Vec::new(),
        );
    }

    let mut failures = Vec::new();
    for candidate in candidates {
        match run_codex_process(&candidate.path, &["--version"], VERSION_TIMEOUT) {
            Ok(output) if output.success => {
                let version = first_nonempty_line(&output.stdout)
                    .or_else(|| first_nonempty_line(&output.stderr));
                let path = path_to_string(&candidate.path);
                return (
                    CodexStatus {
                        installed: true,
                        path: Some(path),
                        version,
                        source: Some(candidate.detected_via.to_owned()),
                    },
                    Some(candidate.path),
                    Vec::new(),
                );
            }
            Ok(output) => failures.push(format!(
                "{} exited with {}{}",
                candidate.path.display(),
                output
                    .code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "an unknown status".to_owned()),
                concise_stderr(&output.stderr)
            )),
            Err(error) => failures.push(format!("{}: {error}", candidate.path.display())),
        }
    }

    (
        CodexStatus {
            installed: false,
            path: None,
            version: None,
            source: None,
        },
        None,
        vec![format!(
            "Codex CLI candidates could not be started: {}",
            failures.join("; ")
        )],
    )
}

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
        path: string_field(
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
        }),
        source: InventorySource::Command,
        error: string_field(object, &["error"]),
    })
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

    Some(SkillItem {
        id,
        name,
        description,
        origin,
        plugin_name: plugin.map(|plugin| plugin.name.clone()),
        path: path_to_string(skill_directory),
        source: InventorySource::Filesystem,
        error: None,
    })
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
        };
        let value = serde_json::to_value(skill).unwrap();

        assert_eq!(value["pluginName"], Value::Null);
        assert_eq!(value["origin"], "personal");
        assert_eq!(value["source"], "filesystem");
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
