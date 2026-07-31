import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type InventorySource = "filesystem" | "config" | "marketplace";

export interface CodexStatus {
  installed: boolean;
  path: string | null;
  version: string | null;
  source: string | null;
}

export interface CodexRuntimeStatus {
  state: "stopped" | "unmanaged" | "managed";
  running: boolean;
  managed: boolean;
  restartRequired: boolean;
}

export interface CodexLaunchProgress {
  stage: "cleaning" | "starting" | "connecting" | "injecting" | "complete";
  message: string;
}

export interface PluginItem {
  id: string;
  name: string;
  version: string | null;
  description: string | null;
  enabled: boolean;
  marketplace: string | null;
  path: string | null;
  source: InventorySource;
  error: string | null;
  icon: string | null;
  official: boolean;
  canDelete: boolean;
}

export interface SkillItem {
  id: string;
  name: string;
  description: string | null;
  origin: "personal" | "plugin" | "system" | "unknown";
  pluginName: string | null;
  path: string;
  source: InventorySource;
  error: string | null;
  icon: string | null;
  pluginIcon: string | null;
  official: boolean;
  canDelete: boolean;
}

export interface AppSnapshot {
  appVersion: string;
  codex: CodexStatus;
  codexRuntime: CodexRuntimeStatus;
  codexHome: string;
  plugins: PluginItem[];
  skills: SkillItem[];
  warnings: string[];
  checkedAt: string;
}

export interface InstallProgress {
  stage: "preparing" | "proxy" | "downloading" | "installing" | "verifying" | "complete" | "error";
  percent: number;
  message: string;
}

export interface CodexUpdateInfo {
  available: boolean;
}

export interface UpdateInfo {
  currentVersion: string;
  version: string;
  notes: string | null;
  publishedAt: string | null;
}

export interface UpdateProgress {
  stage: "proxy" | "checking" | "downloading" | "verifying" | "complete" | "error";
  percent: number;
  message: string;
}

const previewSnapshot: AppSnapshot = {
  appVersion: "0.1.0",
  codex: {
    installed: true,
    path: "C:\\Program Files\\WindowsApps\\OpenAI.Codex_26.721.4979.0_x64__2p2nqsd0c76g0",
    version: "26.721.4979.0",
    source: "Microsoft Store",
  },
  codexRuntime: {
    state: "stopped",
    running: false,
    managed: false,
    restartRequired: false,
  },
  codexHome: "C:\\Users\\jiahao\\.codex",
  plugins: [
    {
      id: "openai-developer-docs@openai-bundled",
      name: "openai-developer-docs",
      version: "1.4.2",
      description: "OpenAI 官方开发文档与 API 参考",
      enabled: true,
      marketplace: "openai-bundled",
      path: "C:\\Users\\jiahao\\.codex\\plugins\\cache\\openai-bundled\\openai-developer-docs\\1.4.2",
      source: "filesystem",
      error: null,
      icon: null,
      official: true,
      canDelete: false,
    },
    {
      id: "team-review@workspace",
      name: "team-review",
      version: "0.8.0",
      description: "团队代码审查工作流",
      enabled: false,
      marketplace: "workspace",
      path: "C:\\Users\\jiahao\\.codex\\plugins\\cache\\workspace\\team-review\\0.8.0",
      source: "filesystem",
      error: null,
      icon: null,
      official: false,
      canDelete: true,
    },
  ],
  skills: [
    {
      id: "openai-docs",
      name: "openai-docs",
      description: "检索并引用 OpenAI 官方文档",
      origin: "system",
      pluginName: null,
      path: "C:\\Users\\jiahao\\.codex\\skills\\.system\\openai-docs",
      source: "filesystem",
      error: null,
      icon: null,
      pluginIcon: null,
      official: true,
      canDelete: false,
    },
    {
      id: "release-review",
      name: "release-review",
      description: "检查发布变更、风险和回滚条件",
      origin: "personal",
      pluginName: null,
      path: "C:\\Users\\jiahao\\.codex\\skills\\release-review",
      source: "filesystem",
      error: null,
      icon: null,
      pluginIcon: null,
      official: false,
      canDelete: true,
    },
    {
      id: "team-review/security-pass",
      name: "security-pass",
      description: "执行团队安全检查清单",
      origin: "plugin",
      pluginName: "team-review",
      path: "C:\\Users\\jiahao\\.codex\\plugins\\cache\\workspace\\team-review\\0.8.0\\skills\\security-pass",
      source: "filesystem",
      error: null,
      icon: null,
      pluginIcon: null,
      official: false,
      canDelete: true,
    },
  ],
  warnings: [],
  checkedAt: new Date().toISOString(),
};

function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

export async function getSnapshot(): Promise<AppSnapshot> {
  if (!isTauri()) return previewSnapshot;
  return invoke<AppSnapshot>("get_snapshot");
}

export async function refreshSnapshot(): Promise<AppSnapshot> {
  if (!isTauri()) {
    await new Promise((resolve) => window.setTimeout(resolve, 450));
    return { ...previewSnapshot, checkedAt: new Date().toISOString() };
  }
  return invoke<AppSnapshot>("refresh_snapshot");
}

export async function installCodex(): Promise<void> {
  if (!isTauri()) return;
  await invoke("install_codex");
}

export async function updateCodex(): Promise<void> {
  if (!isTauri()) return;
  await invoke("update_codex");
}

export async function launchCodex(codex: CodexStatus): Promise<CodexRuntimeStatus> {
  if (!isTauri()) return { state: "managed", running: true, managed: true, restartRequired: false };
  return invoke<CodexRuntimeStatus>("launch_codex", { codex });
}

export async function checkCodexUpdate(): Promise<CodexUpdateInfo> {
  if (!isTauri()) return { available: false };
  return invoke<CodexUpdateInfo>("check_codex_update");
}

export async function revealPath(path: string): Promise<void> {
  if (!isTauri()) return;
  await invoke("reveal_path", { path });
}

export async function deletePlugin(pluginId: string): Promise<void> {
  if (!isTauri()) return;
  await invoke("delete_plugin", { pluginId });
}

export async function deleteSkill(skillId: string): Promise<void> {
  if (!isTauri()) return;
  await invoke("delete_skill", { skillId });
}

export async function readSkillContent(skillId: string): Promise<string> {
  if (!isTauri()) {
    return "# Skill 内容\n\n当前为浏览器预览模式。请在 Codex Go 桌面端查看完整的 SKILL.md。";
  }
  return invoke<string>("read_skill_content", { skillId });
}

export async function checkAppUpdate(): Promise<UpdateInfo | null> {
  if (!isTauri()) return null;
  return invoke<UpdateInfo | null>("check_app_update");
}

export async function installAppUpdate(): Promise<void> {
  if (!isTauri()) return;
  await invoke("install_app_update");
}

export async function onInstallProgress(
  callback: (progress: InstallProgress) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<InstallProgress>("install-progress", (event) => callback(event.payload));
}

export async function onCodexLaunchProgress(
  callback: (progress: CodexLaunchProgress) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<CodexLaunchProgress>("codex-launch-progress", (event) => callback(event.payload));
}

export async function onAppUpdateProgress(
  callback: (progress: UpdateProgress) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<UpdateProgress>("app-update-progress", (event) => callback(event.payload));
}
