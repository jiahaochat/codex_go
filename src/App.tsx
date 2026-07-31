import { Fragment, useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  Box,
  Check,
  Clipboard,
  ChevronRight,
  CircleAlert,
  CloudDownload,
  Copy,
  Download,
  FolderOpen,
  FileText,
  Gauge,
  Layers3,
  LoaderCircle,
  Minus,
  PackageCheck,
  Play,
  Plug,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
  Square,
  Sparkles,
  TerminalSquare,
  Trash2,
  X,
} from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import codexGoIcon from "../assets/codex-go-icon.png";
import {
  checkAppUpdate,
  checkCodexUpdate,
  deletePlugin,
  deleteSkill,
  getSnapshot,
  installAppUpdate,
  installCodex,
  launchCodex,
  updateCodex,
  onAppUpdateProgress,
  onCodexLaunchProgress,
  onInstallProgress,
  refreshSnapshot,
  readSkillContent,
  revealPath,
  type AppSnapshot,
  type CodexLaunchProgress,
  type InstallProgress,
  type PluginItem,
  type SkillItem,
  type UpdateInfo,
  type UpdateProgress,
} from "./api";
import {
  filterPlugins,
  filterSkills,
  groupSkillsByPlugin,
  type PluginFilter,
  type SkillFilter,
} from "./inventory-filter";

type View = "overview" | "plugins" | "skills" | "settings";
type StatusTone = "success" | "warning" | "neutral";

const sourceLabels = {
  filesystem: "本地目录",
  config: "配置文件",
  marketplace: "插件市场",
};

const originLabels = {
  personal: "个人",
  plugin: "插件附带",
  system: "系统",
  unknown: "其他",
};

const runtimeLabels = {
  stopped: "未运行",
  unmanaged: "未通过 Codex Go 运行",
  managed: "已运行",
};

function launchButtonLabel(progress: CodexLaunchProgress | null): string {
  return {
    cleaning: "正在清理",
    starting: "正在启动",
    connecting: "正在连接",
    injecting: "正在注入",
    complete: "已运行",
  }[progress?.stage ?? "starting"];
}

function runButtonLabel(state: AppSnapshot["codexRuntime"]["state"]): string {
  if (state === "managed") return "已运行";
  if (state === "unmanaged") return "从 Codex Go 运行";
  return "运行";
}

function App() {
  const [view, setView] = useState<View>("overview");
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<InstallProgress | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);
  const [launching, setLaunching] = useState(false);
  const [launchProgress, setLaunchProgress] = useState<CodexLaunchProgress | null>(null);
  const [launchError, setLaunchError] = useState<string | null>(null);
  const [checkingCodexUpdate, setCheckingCodexUpdate] = useState(false);
  const [codexUpdateAvailable, setCodexUpdateAvailable] = useState(false);
  const [codexUpdateChecked, setCodexUpdateChecked] = useState(false);
  const [availableUpdate, setAvailableUpdate] = useState<UpdateInfo | null>(null);
  const [updateDismissed, setUpdateDismissed] = useState(false);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [updateChecked, setUpdateChecked] = useState(false);
  const [updating, setUpdating] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<UpdateProgress | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);

  const load = useCallback(async (refresh = false) => {
    refresh ? setRefreshing(true) : setLoading(true);
    setError(null);
    try {
      const next = refresh ? await refreshSnapshot() : await getSnapshot();
      setSnapshot(next);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    setCodexUpdateAvailable(false);
    setCodexUpdateChecked(false);
  }, [snapshot?.codex.version]);

  const checkUpdates = useCallback(async (manual = false) => {
    setCheckingUpdate(true);
    setUpdateChecked(false);
    if (manual) setUpdateError(null);
    try {
      const next = await checkAppUpdate();
      setAvailableUpdate(next);
      setUpdateDismissed(false);
      setUpdateChecked(true);
    } catch (reason) {
      if (manual) setUpdateError(errorMessage(reason));
    } finally {
      setCheckingUpdate(false);
    }
  }, []);

  useEffect(() => {
    void checkUpdates();
  }, [checkUpdates]);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    void onAppUpdateProgress((next) => {
      setUpdateProgress(next);
      if (next.stage === "complete" || next.stage === "error") setUpdating(false);
    }).then((unlisten) => {
      dispose = unlisten;
    });
    return () => dispose?.();
  }, []);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    void onInstallProgress((next) => {
      setProgress(next);
      if (next.stage === "complete" || next.stage === "error") {
        setInstalling(false);
        if (next.stage === "complete") {
          setInstallError(null);
          void load(true);
        } else {
          setInstallError(next.message);
        }
      }
    }).then((unlisten) => {
      dispose = unlisten;
    });
    return () => dispose?.();
  }, [load]);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    void onCodexLaunchProgress(setLaunchProgress).then((unlisten) => {
      dispose = unlisten;
    });
    return () => dispose?.();
  }, []);

  const startCodexAction = async () => {
    if (updating) {
      setInstallError("请等待软件更新任务结束");
      return;
    }
    const isInstalled = Boolean(snapshot?.codex.installed);
    setInstallError(null);

    if (isInstalled && !codexUpdateAvailable) {
      setCheckingCodexUpdate(true);
      try {
        const result = await checkCodexUpdate();
        setCodexUpdateAvailable(result.available);
        setCodexUpdateChecked(true);
      } catch (reason) {
        setInstallError(errorMessage(reason));
      } finally {
        setCheckingCodexUpdate(false);
      }
      return;
    }

    setInstalling(true);
    setProgress({ stage: "preparing", percent: 4, message: isInstalled ? "正在准备 Codex Windows 更新" : "正在准备 Codex Windows 安装" });
    try {
      if (isInstalled) {
        await updateCodex();
        setCodexUpdateAvailable(false);
        setCodexUpdateChecked(true);
      }
      else await installCodex();
    } catch (reason) {
      const message = errorMessage(reason);
      setInstalling(false);
      setInstallError(message);
      setProgress({ stage: "error", percent: 100, message });
    }
  };

  const startCodex = async () => {
    if (!snapshot?.codex.installed) {
      setLaunchError("请先安装 Codex Windows 桌面端");
      return;
    }
    if (installing || checkingCodexUpdate || updating) {
      setLaunchError("请等待当前安装或更新任务结束");
      return;
    }

    setLaunching(true);
    setLaunchProgress({ stage: "starting", message: "正在启动 Codex Windows 桌面端" });
    setLaunchError(null);
    try {
      const runtime = await launchCodex(snapshot.codex);
      setSnapshot((current) => current ? { ...current, codexRuntime: runtime } : current);
    } catch (reason) {
      setLaunchError(errorMessage(reason));
    } finally {
      setLaunching(false);
    }
  };

  const startAppUpdate = async () => {
    if (installing || checkingCodexUpdate || launching) {
      setUpdateError("请等待 Codex 操作结束");
      return;
    }
    setUpdating(true);
    setUpdateError(null);
    setUpdateProgress({ stage: "proxy", percent: 3, message: "正在准备更新" });
    try {
      await installAppUpdate();
    } catch (reason) {
      const message = errorMessage(reason);
      setUpdating(false);
      setUpdateError(message);
      setUpdateProgress({ stage: "error", percent: 100, message });
    }
  };

  const title = {
    overview: "总览",
    plugins: "插件",
    skills: "Skills",
    settings: "应用设置",
  }[view];

  return (
    <div className="app-frame">
      <WindowTitlebar />
      <div className="app-shell">
        <Sidebar view={view} onChange={setView} snapshot={snapshot} />
        <main className="main-panel">
          <header className="topbar">
            <div>
              <p className="eyebrow">本机 Codex 管理</p>
              <h1>{title}</h1>
            </div>
            <div className="topbar-actions">
              {snapshot && <span className="checked-at">更新于 {formatTime(snapshot.checkedAt)}</span>}
              <button
                className="icon-button"
                type="button"
                title="刷新本机状态"
                aria-label="刷新本机状态"
                disabled={refreshing}
                onClick={() => void load(true)}
              >
                <RefreshCw size={18} className={refreshing ? "spin" : undefined} />
              </button>
            </div>
          </header>

          <div className="content">
            {loading && !snapshot ? <LoadingState /> : null}
            {error ? <ErrorBanner message={error} onRetry={() => void load(true)} /> : null}
            {availableUpdate && !updateDismissed ? (
              <UpdateBanner
                update={availableUpdate}
                updating={updating}
                blocked={installing || checkingCodexUpdate || launching}
                progress={updateProgress}
                onInstall={() => void startAppUpdate()}
                onDismiss={() => setUpdateDismissed(true)}
              />
            ) : null}
            {launchError ? <ErrorBanner message={launchError} onRetry={() => void startCodex()} /> : null}
            {installError ? <ErrorBanner message={installError} onRetry={() => void startCodexAction()} /> : null}
            {updateError ? <ErrorBanner message={updateError} onRetry={() => void checkUpdates(true)} /> : null}
            {snapshot && view === "overview" ? (
              <Overview
                snapshot={snapshot}
                launching={launching}
                launchProgress={launchProgress}
                installing={installing}
                checkingUpdate={checkingCodexUpdate}
                updateAvailable={codexUpdateAvailable}
                updateChecked={codexUpdateChecked}
                operationBusy={installing || checkingCodexUpdate || updating || launching}
                progress={progress}
                onInstall={() => void startCodexAction()}
                onRun={() => void startCodex()}
                onNavigate={setView}
              />
            ) : null}
            {snapshot && view === "plugins" ? <PluginInventory items={snapshot.plugins} onChanged={() => load(true)} /> : null}
            {snapshot && view === "skills" ? <SkillInventory items={snapshot.skills} onChanged={() => load(true)} /> : null}
            {snapshot && view === "settings" ? (
              <SettingsView
                snapshot={snapshot}
                launching={launching}
                launchProgress={launchProgress}
                installing={installing}
                checkingCodexUpdate={checkingCodexUpdate}
                codexUpdateAvailable={codexUpdateAvailable}
                codexUpdateChecked={codexUpdateChecked}
                operationBusy={installing || checkingCodexUpdate || updating || launching}
                progress={progress}
                onInstall={() => void startCodexAction()}
                onRun={() => void startCodex()}
                update={availableUpdate}
                updateProgress={updateProgress}
                checkingUpdate={checkingUpdate}
                updateChecked={updateChecked}
                updating={updating}
                onCheckUpdate={() => void checkUpdates(true)}
                onInstallUpdate={() => void startAppUpdate()}
              />
            ) : null}
          </div>
        </main>
      </div>
    </div>
  );
}

function WindowTitlebar() {
  const [maximized, setMaximized] = useState(false);
  const tauriRuntime = "__TAURI_INTERNALS__" in window;
  const appWindow = useMemo(() => tauriRuntime ? getCurrentWindow() : null, [tauriRuntime]);

  useEffect(() => {
    if (!appWindow) return;

    let disposed = false;
    let unlisten: (() => void) | undefined;
    const syncMaximized = () => {
      void appWindow.isMaximized().then((value) => {
        if (!disposed) setMaximized(value);
      });
    };

    syncMaximized();
    void appWindow.onResized(syncMaximized).then((next) => {
      if (disposed) next();
      else unlisten = next;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [appWindow]);

  const run = (command: () => Promise<void>) => {
    if (appWindow) void command();
  };

  return (
    <header className="window-titlebar" data-tauri-drag-region>
      <div className="titlebar-brand" data-tauri-drag-region>
        <img src={codexGoIcon} alt="" aria-hidden="true" />
        <strong data-tauri-drag-region>codex_go</strong>
      </div>
      <div
        className="titlebar-drag-area"
        data-tauri-drag-region
        onDoubleClick={() => appWindow && run(() => appWindow.toggleMaximize())}
      />
      <div className="window-controls">
        <button type="button" title="最小化" aria-label="最小化窗口" onClick={() => appWindow && run(() => appWindow.minimize())}>
          <Minus size={16} strokeWidth={1.7} />
        </button>
        <button
          type="button"
          title={maximized ? "还原" : "最大化"}
          aria-label={maximized ? "还原窗口" : "最大化窗口"}
          onClick={() => appWindow && run(() => appWindow.toggleMaximize())}
        >
          {maximized ? <Copy size={13} strokeWidth={1.5} /> : <Square size={13} strokeWidth={1.5} />}
        </button>
        <button className="window-close" type="button" title="关闭" aria-label="关闭窗口" onClick={() => appWindow && run(() => appWindow.close())}>
          <X size={17} strokeWidth={1.7} />
        </button>
      </div>
    </header>
  );
}

function Sidebar({
  view,
  onChange,
  snapshot,
}: {
  view: View;
  onChange: (view: View) => void;
  snapshot: AppSnapshot | null;
}) {
  const navigation: Array<{ id: View; label: string; icon: typeof Gauge; count?: number }> = [
    { id: "overview", label: "总览", icon: Gauge },
    { id: "plugins", label: "插件", icon: Plug, count: snapshot?.plugins.length },
    { id: "skills", label: "Skills", icon: Sparkles, count: snapshot?.skills.length },
    { id: "settings", label: "应用设置", icon: Settings2 },
  ];

  return (
    <aside className="sidebar">
      <nav aria-label="主要导航">
        {navigation.map(({ id, label, icon: Icon, count }) => (
          <button
            key={id}
            className={view === id ? "nav-item active" : "nav-item"}
            type="button"
            onClick={() => onChange(id)}
          >
            <Icon size={18} />
            <span>{label}</span>
            {count !== undefined ? <b>{count}</b> : null}
          </button>
        ))}
      </nav>
      <div className="sidebar-footer">
        <span className={snapshot?.codexRuntime.state === "managed" ? "status-dot online" : "status-dot"} />
        <div>
          <strong>{snapshot ? runtimeLabels[snapshot.codexRuntime.state] : "正在检测"}</strong>
          <span>{snapshot?.codexRuntime.state === "managed" ? `Codex Go ${snapshot.appVersion}` : snapshot?.codex.version ?? "尚未检测到版本"}</span>
        </div>
      </div>
    </aside>
  );
}

function Overview({
  snapshot,
  launching,
  launchProgress,
  installing,
  checkingUpdate,
  updateAvailable,
  updateChecked,
  operationBusy,
  progress,
  onInstall,
  onRun,
  onNavigate,
}: {
  snapshot: AppSnapshot;
  launching: boolean;
  launchProgress: CodexLaunchProgress | null;
  installing: boolean;
  checkingUpdate: boolean;
  updateAvailable: boolean;
  updateChecked: boolean;
  operationBusy: boolean;
  progress: InstallProgress | null;
  onInstall: () => void;
  onRun: () => void;
  onNavigate: (view: View) => void;
}) {
  return (
    <div className="view-stack">
      <CodexHero
        snapshot={snapshot}
        launching={launching}
        launchProgress={launchProgress}
        installing={installing}
        checkingUpdate={checkingUpdate}
        updateAvailable={updateAvailable}
        updateChecked={updateChecked}
        operationBusy={operationBusy}
        progress={progress}
        onInstall={onInstall}
        onRun={onRun}
      />

      {snapshot.warnings.length ? (
        <div className="warning-list">
          {snapshot.warnings.map((warning) => (
            <div className="warning-row" key={warning}>
              <CircleAlert size={17} />
              <span>{warning}</span>
            </div>
          ))}
        </div>
      ) : null}

      {snapshot.codexRuntime.state === "unmanaged" ? (
        <div className="warning-row launch-warning">
          <CircleAlert size={17} />
          <span>当前 Codex 桌面端不是通过 Codex Go 启动。点击“从 Codex Go 运行”后将自动关闭当前 Codex 并重新启动。</span>
        </div>
      ) : null}

      <section className="metrics-grid" aria-label="本机扩展摘要">
        <Metric
          icon={Plug}
          label="已安装插件"
          value={snapshot.plugins.length}
          detail={`${snapshot.plugins.filter((item) => item.enabled).length} 个已启用`}
          accent="cyan"
          onClick={() => onNavigate("plugins")}
        />
        <Metric
          icon={Sparkles}
          label="可用 Skills"
          value={snapshot.skills.length}
          detail={`${snapshot.skills.filter((item) => item.origin === "personal").length} 个个人 Skill`}
          accent="amber"
          onClick={() => onNavigate("skills")}
        />
      </section>

      <section className="section-block">
        <div className="section-heading">
          <div>
            <p className="eyebrow">最近识别</p>
            <h2>本机扩展</h2>
          </div>
          <button className="text-command" type="button" onClick={() => onNavigate("plugins")}>
            查看全部 <ChevronRight size={16} />
          </button>
        </div>
        <div className="activity-list">
          {snapshot.plugins.slice(0, 3).map((plugin) => (
            <InventoryRow
              key={plugin.id}
              icon={Box}
              iconSrc={plugin.icon}
              name={plugin.name}
              description={plugin.description ?? plugin.marketplace ?? "Codex 插件"}
              meta={plugin.version ? `v${plugin.version}` : sourceLabels[plugin.source]}
              status={plugin.enabled ? "已启用" : "未启用"}
              tone={plugin.enabled ? "success" : "neutral"}
              path={plugin.path}
              official={plugin.official}
            />
          ))}
          {!snapshot.plugins.length ? <EmptyState title="尚未识别到插件" compact /> : null}
        </div>
      </section>
    </div>
  );
}

function CodexHero({
  snapshot,
  launching,
  launchProgress,
  installing,
  checkingUpdate,
  updateAvailable,
  updateChecked,
  operationBusy,
  progress,
  onInstall,
  onRun,
}: {
  snapshot: AppSnapshot;
  launching: boolean;
  launchProgress: CodexLaunchProgress | null;
  installing: boolean;
  checkingUpdate: boolean;
  updateAvailable: boolean;
  updateChecked: boolean;
  operationBusy: boolean;
  progress: InstallProgress | null;
  onInstall: () => void;
  onRun: () => void;
}) {
  const installed = snapshot.codex.installed;
  const runtime = snapshot.codexRuntime;
  return (
    <section className={installed ? "codex-hero installed" : "codex-hero missing"}>
      <div className="hero-icon">
        {installed ? <TerminalSquare size={29} /> : <Download size={29} />}
      </div>
      <div className="hero-copy">
        <div className="hero-title-line">
          <h2>{installed ? "Codex Windows 桌面端已安装" : "尚未安装 Codex Windows 桌面端"}</h2>
          <StatusPill tone={!installed || runtime.state === "unmanaged" ? "warning" : runtime.state === "managed" ? "success" : "neutral"}>
            {installed ? runtimeLabels[runtime.state] : "需要安装"}
          </StatusPill>
        </div>
        <p>
          {installed
            ? snapshot.codex.path ?? "已通过 Microsoft Store 识别 Codex Windows 桌面端"
            : "从 Microsoft Store 安装 OpenAI 官方 Codex Windows 桌面端。"}
        </p>
        <div className="hero-meta">
          <span><FolderOpen size={15} /> {snapshot.codexHome}</span>
          <span><ShieldCheck size={15} /> Microsoft Store 官方分发</span>
        </div>
        {updateAvailable ? (
          <div className="hero-update-notice"><CircleAlert size={15} /> Codex Windows 桌面端有新版本，请点击“版本更新”完成升级。</div>
        ) : installed && updateChecked ? (
          <div className="hero-current-notice"><Check size={15} /> 当前已是 Microsoft Store 提供的最新版本。</div>
        ) : null}
        {runtime.state === "managed" ? (
          <div className="hero-current-notice"><Check size={15} /> 已注入 Codex Go {snapshot.appVersion} 版本标识。</div>
        ) : runtime.state === "unmanaged" ? (
          <div className="hero-update-notice"><CircleAlert size={15} /> 点击“从 Codex Go 运行”将自动关闭当前 Codex 并通过本软件重新启动。</div>
        ) : null}
        {installing && progress ? (
          <div className="install-progress" aria-live="polite">
            <div className="progress-label"><span>{progress.message}</span><b>{progress.percent}%</b></div>
            <div className="progress-track"><i style={{ width: `${progress.percent}%` }} /></div>
          </div>
        ) : null}
      </div>
      <div className="hero-action">
        {installed ? (
          <>
            <button className="primary-command" type="button" onClick={onRun} disabled={operationBusy}>
              {launching ? <LoaderCircle size={17} className="spin" /> : <Play size={17} />}
              {launching ? launchButtonLabel(launchProgress) : runButtonLabel(runtime.state)}
            </button>
            <button className="secondary-command" type="button" onClick={onInstall} disabled={operationBusy}>
              {installing || checkingUpdate ? <LoaderCircle size={17} className="spin" /> : updateAvailable ? <CloudDownload size={17} /> : <RefreshCw size={17} />}
              {installing ? "正在更新" : checkingUpdate ? "正在检查" : updateAvailable ? "版本更新" : "检查更新"}
            </button>
          </>
        ) : (
          <button className="primary-command" type="button" onClick={onInstall} disabled={operationBusy}>
            {installing ? <LoaderCircle size={17} className="spin" /> : <Download size={17} />}
            {installing ? "正在安装" : "安装 Codex"}
          </button>
        )}
      </div>
    </section>
  );
}

function PluginInventory({ items, onChanged }: { items: PluginItem[]; onChanged: () => Promise<void> }) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<PluginFilter>("all");
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const visible = useMemo(() => filterPlugins(items, query, filter), [filter, items, query]);

  const remove = async (plugin: PluginItem) => {
    if (!window.confirm(`确定删除插件“${plugin.name}”吗？此操作会移除它的所有本地版本。`)) return;
    setDeletingId(plugin.id);
    setDeleteError(null);
    try {
      await deletePlugin(plugin.id);
      await onChanged();
    } catch (reason) {
      setDeleteError(errorMessage(reason));
    } finally {
      setDeletingId(null);
    }
  };

  return (
    <InventoryView
      label="插件库存"
      title={`${items.length} 个已安装插件`}
      query={query}
      onQuery={setQuery}
      filter={filter}
      onFilter={(value) => setFilter(value as typeof filter)}
      filters={[
        ["all", "全部"],
        ["enabled", "已启用"],
        ["disabled", "未启用"],
      ]}
    >
      {deleteError ? <InventoryError message={deleteError} /> : null}
      {visible.map((plugin) => (
        <InventoryRow
          key={plugin.id}
          icon={PackageCheck}
          iconSrc={plugin.icon}
          name={plugin.name}
          description={plugin.description ?? `${plugin.marketplace ?? "本地"} 插件`}
          meta={[plugin.version ? `v${plugin.version}` : null, sourceLabels[plugin.source]].filter(Boolean).join(" · ")}
          status={plugin.enabled ? "已启用" : "未启用"}
          tone={plugin.enabled ? "success" : "neutral"}
          path={plugin.path}
          official={plugin.official}
          deleting={deletingId === plugin.id}
          onDelete={plugin.canDelete ? () => void remove(plugin) : undefined}
        />
      ))}
      {!visible.length ? <EmptyState title={items.length ? "没有符合条件的插件" : "尚未识别到插件"} /> : null}
    </InventoryView>
  );
}

function SkillInventory({ items, onChanged }: { items: SkillItem[]; onChanged: () => Promise<void> }) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<SkillFilter>("all");
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(() => new Set());
  const [selectedSkill, setSelectedSkill] = useState<SkillItem | null>(null);
  const [skillContent, setSkillContent] = useState("");
  const [skillLoading, setSkillLoading] = useState(false);
  const [skillError, setSkillError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const visible = useMemo(() => filterSkills(items, query, filter), [filter, items, query]);
  const grouped = useMemo(() => groupSkillsByPlugin(visible), [visible]);

  const remove = async (skill: SkillItem) => {
    if (!window.confirm(`确定删除 Skill“${skill.name}”吗？此操作无法撤销。`)) return;
    setDeletingId(skill.id);
    setDeleteError(null);
    try {
      await deleteSkill(skill.id);
      await onChanged();
    } catch (reason) {
      setDeleteError(errorMessage(reason));
    } finally {
      setDeletingId(null);
    }
  };

  const toggleGroup = (pluginName: string) => {
    const key = pluginName.toLocaleLowerCase("zh-CN");
    setExpandedGroups((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const openSkill = async (skill: SkillItem) => {
    setSelectedSkill(skill);
    setSkillContent("");
    setSkillError(null);
    setCopied(false);
    setSkillLoading(true);
    try {
      setSkillContent(await readSkillContent(skill.id));
    } catch (reason) {
      setSkillError(errorMessage(reason));
    } finally {
      setSkillLoading(false);
    }
  };

  const copySkill = async () => {
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(skillContent);
      } else {
        const textarea = document.createElement("textarea");
        textarea.value = skillContent;
        textarea.style.position = "fixed";
        textarea.style.opacity = "0";
        document.body.appendChild(textarea);
        textarea.select();
        if (!document.execCommand("copy")) throw new Error("copy failed");
        textarea.remove();
      }
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      setSkillError("复制失败，请手动选择内容复制");
    }
  };

  const renderSkill = (skill: SkillItem) => (
    <InventoryRow
      key={skill.id}
      icon={Sparkles}
      iconSrc={skill.icon}
      name={skill.name}
      description={skill.description ?? "暂无描述"}
      meta=""
      status=""
      tone={skill.origin === "personal" ? "success" : "neutral"}
      path={skill.path}
      official={skill.official}
      deleting={deletingId === skill.id}
      showStatus={false}
      onOpen={() => void openSkill(skill)}
      onDelete={skill.canDelete ? () => void remove(skill) : undefined}
    />
  );

  return (
    <>
      <InventoryView
      label="Skill 库存"
      title={`${items.length} 个可用 Skills`}
      query={query}
      onQuery={setQuery}
      filter={filter}
      onFilter={(value) => setFilter(value as typeof filter)}
      filters={[
        ["all", "全部"],
        ["personal", "个人"],
        ["plugin", "插件附带"],
        ["system", "系统"],
      ]}
      >
      {deleteError ? <InventoryError message={deleteError} /> : null}
      {grouped.standalone.map(renderSkill)}
      {grouped.pluginGroups.map((group, index) => {
        const groupKey = group.pluginName.toLocaleLowerCase("zh-CN");
        const expanded = Boolean(query.trim()) || expandedGroups.has(groupKey);
        const contentId = `plugin-skill-group-${index}`;
        const official = group.items.every((skill) => skill.official);

        return (
          <div className={expanded ? "skill-group expanded" : "skill-group"} key={groupKey}>
            <button
              className="skill-group-header"
              type="button"
              aria-expanded={expanded}
              aria-controls={contentId}
              onClick={() => toggleGroup(group.pluginName)}
            >
              <span className="skill-group-chevron"><ChevronRight size={16} /></span>
              <span className={group.items[0].pluginIcon ? "skill-group-icon has-image" : "skill-group-icon"}>
                {group.items[0].pluginIcon ? <img src={group.items[0].pluginIcon} alt="" aria-hidden="true" /> : <PackageCheck size={17} />}
              </span>
              <span className="skill-group-copy">
                <strong>{group.pluginName}</strong>
                <span>{group.items.length} 个插件附带 Skill</span>
              </span>
              {official ? <span className="official-badge"><ShieldCheck size={12} />官方</span> : null}
              <span className="skill-group-count">{group.items.length}</span>
            </button>
            {expanded ? <div className="skill-group-items" id={contentId}>{group.items.map(renderSkill)}</div> : null}
          </div>
        );
      })}
      {!visible.length ? <EmptyState title={items.length ? "没有符合条件的 Skill" : "尚未识别到 Skill"} /> : null}
      </InventoryView>
      {selectedSkill ? (
        <SkillDetailDialog
          skill={selectedSkill}
          content={skillContent}
          loading={skillLoading}
          error={skillError}
          copied={copied}
          onCopy={() => void copySkill()}
          onClose={() => setSelectedSkill(null)}
        />
      ) : null}
    </>
  );
}

function InventoryView({
  label,
  title,
  query,
  onQuery,
  filter,
  onFilter,
  filters,
  children,
}: {
  label: string;
  title: string;
  query: string;
  onQuery: (value: string) => void;
  filter: string;
  onFilter: (value: string) => void;
  filters: string[][];
  children: ReactNode;
}) {
  return (
    <div className="view-stack">
      <section className="inventory-header">
        <div>
          <p className="eyebrow">{label}</p>
          <h2>{title}</h2>
        </div>
        <label className="search-box">
          <Search size={17} />
          <input value={query} onChange={(event) => onQuery(event.target.value)} placeholder="搜索名称、描述或来源" />
          {query ? (
            <button type="button" title="清空搜索" aria-label="清空搜索" onClick={() => onQuery("")}>
              <X size={15} />
            </button>
          ) : null}
        </label>
      </section>
      <div className="segmented-control" role="tablist" aria-label="库存筛选">
        {filters.map(([value, text]) => (
          <button
            key={value}
            type="button"
            role="tab"
            aria-selected={filter === value}
            className={filter === value ? "selected" : undefined}
            onClick={() => onFilter(value)}
          >
            {text}
          </button>
        ))}
      </div>
      <section className="inventory-table">{children}</section>
    </div>
  );
}

function SettingsView({
  snapshot,
  launching,
  launchProgress,
  installing,
  checkingCodexUpdate,
  codexUpdateAvailable,
  codexUpdateChecked,
  operationBusy,
  progress,
  onInstall,
  onRun,
  update,
  updateProgress,
  checkingUpdate,
  updateChecked,
  updating,
  onCheckUpdate,
  onInstallUpdate,
}: {
  snapshot: AppSnapshot;
  launching: boolean;
  launchProgress: CodexLaunchProgress | null;
  installing: boolean;
  checkingCodexUpdate: boolean;
  codexUpdateAvailable: boolean;
  codexUpdateChecked: boolean;
  operationBusy: boolean;
  progress: InstallProgress | null;
  onInstall: () => void;
  onRun: () => void;
  update: UpdateInfo | null;
  updateProgress: UpdateProgress | null;
  checkingUpdate: boolean;
  updateChecked: boolean;
  updating: boolean;
  onCheckUpdate: () => void;
  onInstallUpdate: () => void;
}) {
  return (
    <div className="view-stack settings-stack">
      <section className="settings-section">
        <div className="settings-heading">
          <div className="settings-icon"><TerminalSquare size={20} /></div>
          <div><h2>Codex Windows 桌面端</h2><p>Microsoft Store 官方安装与本机检测</p></div>
        </div>
        <dl className="details-list">
          <div><dt>状态</dt><dd><StatusPill tone={snapshot.codex.installed ? "success" : "warning"}>{snapshot.codex.installed ? "已安装" : "未安装"}</StatusPill></dd></div>
          <div><dt>运行状态</dt><dd><StatusPill tone={snapshot.codexRuntime.state === "managed" ? "success" : snapshot.codexRuntime.state === "unmanaged" ? "warning" : "neutral"}>{runtimeLabels[snapshot.codexRuntime.state]}</StatusPill></dd></div>
          <div><dt>版本</dt><dd>{snapshot.codex.version ?? "-"}</dd></div>
          {snapshot.codex.installed ? (
            <div><dt>更新状态</dt><dd><StatusPill tone={codexUpdateAvailable ? "warning" : codexUpdateChecked ? "success" : "neutral"}>{checkingCodexUpdate ? "正在检查" : codexUpdateAvailable ? "发现新版本" : codexUpdateChecked ? "已是最新" : "尚未检查"}</StatusPill></dd></div>
          ) : null}
          <div><dt>安装位置</dt><dd className="path-value">{snapshot.codex.path ?? "未检测到"}</dd></div>
          <div><dt>分发渠道</dt><dd>{snapshot.codex.source ?? "-"}</dd></div>
          <div><dt>CODEX_HOME</dt><dd className="path-value">{snapshot.codexHome}</dd></div>
        </dl>
        <div className="command-row">
          {snapshot.codex.installed ? (
            <button className="primary-command" type="button" onClick={onRun} disabled={operationBusy}>
              {launching ? <LoaderCircle size={17} className="spin" /> : <Play size={17} />}
              {launching ? launchButtonLabel(launchProgress) : runButtonLabel(snapshot.codexRuntime.state)}
            </button>
          ) : null}
          <button className={snapshot.codex.installed ? "secondary-command" : "primary-command"} type="button" onClick={onInstall} disabled={operationBusy}>
            {installing || checkingCodexUpdate ? <LoaderCircle size={17} className="spin" /> : codexUpdateAvailable ? <CloudDownload size={17} /> : snapshot.codex.installed ? <RefreshCw size={17} /> : <Download size={17} />}
            {installing ? (snapshot.codex.installed ? progress?.message ?? "正在更新" : progress?.message ?? "正在安装") : checkingCodexUpdate ? "正在检查" : snapshot.codex.installed ? (codexUpdateAvailable ? "版本更新" : "检查更新") : "安装官方 Codex Windows 桌面端"}
          </button>
          {snapshot.codex.installed ? (
            <button className="secondary-command" type="button" onClick={() => snapshot.codex.path && void revealPath(snapshot.codex.path)} disabled={!snapshot.codex.path}>
              <FolderOpen size={17} /> 打开安装位置
            </button>
          ) : null}
        </div>
      </section>

      <section className="settings-section">
        <div className="settings-heading">
          <div className="settings-icon"><CloudDownload size={20} /></div>
          <div><h2>codex_go 更新</h2><p>稳定版 GitHub Releases</p></div>
        </div>
        <dl className="details-list">
          <div><dt>当前版本</dt><dd>v{snapshot.appVersion}</dd></div>
          <div>
            <dt>更新状态</dt>
            <dd>
              <StatusPill tone={update ? "warning" : updateChecked ? "success" : "neutral"}>
                {checkingUpdate ? "正在检查" : update ? `发现 v${update.version}` : updateChecked ? "已是最新" : "尚未检查"}
              </StatusPill>
            </dd>
          </div>
          <div><dt>签名校验</dt><dd>强制启用</dd></div>
        </dl>
        {updating && updateProgress ? (
          <div className="install-progress settings-progress" aria-live="polite">
            <div className="progress-label"><span>{updateProgress.message}</span><b>{updateProgress.percent}%</b></div>
            <div className="progress-track"><i style={{ width: `${updateProgress.percent}%` }} /></div>
          </div>
        ) : null}
        <div className="command-row">
          <button className="secondary-command" type="button" onClick={onCheckUpdate} disabled={checkingUpdate || operationBusy}>
            <RefreshCw size={17} className={checkingUpdate ? "spin" : undefined} /> {checkingUpdate ? "正在检查" : "检查更新"}
          </button>
          {update ? (
            <button className="primary-command" type="button" onClick={onInstallUpdate} disabled={operationBusy}>
              {updating ? <LoaderCircle size={17} className="spin" /> : <CloudDownload size={17} />}
              {updating ? "正在更新" : `更新到 v${update.version}`}
            </button>
          ) : null}
        </div>
      </section>

    </div>
  );
}

function UpdateBanner({ update, updating, blocked, progress, onInstall, onDismiss }: { update: UpdateInfo; updating: boolean; blocked: boolean; progress: UpdateProgress | null; onInstall: () => void; onDismiss: () => void }) {
  return (
    <section className="update-banner" aria-live="polite">
      <div className="update-banner-icon"><CloudDownload size={22} /></div>
      <div className="update-banner-copy">
        <div><strong>codex_go v{update.version} 可更新</strong><span>当前 v{update.currentVersion}</span></div>
        <p>{update.notes || "新的稳定版本已经发布。"}</p>
        {updating && progress ? (
          <div className="install-progress">
            <div className="progress-label"><span>{progress.message}</span><b>{progress.percent}%</b></div>
            <div className="progress-track"><i style={{ width: `${progress.percent}%` }} /></div>
          </div>
        ) : null}
      </div>
      <div className="update-banner-actions">
        <button className="primary-command" type="button" onClick={onInstall} disabled={updating || blocked}>
          {updating ? <LoaderCircle size={17} className="spin" /> : <CloudDownload size={17} />}
          {updating ? "正在更新" : "立即更新"}
        </button>
        <button className="icon-button" type="button" title="稍后更新" aria-label="稍后更新" onClick={onDismiss} disabled={updating}><X size={17} /></button>
      </div>
    </section>
  );
}

function Metric({ icon: Icon, label, value, detail, accent, onClick }: { icon: typeof Plug; label: string; value: string | number; detail: string; accent: string; onClick: () => void }) {
  return (
    <button className="metric" type="button" onClick={onClick}>
      <span className={`metric-icon ${accent}`}><Icon size={20} /></span>
      <span className="metric-copy"><small>{label}</small><strong>{value}</strong><span>{detail}</span></span>
      <ChevronRight size={17} className="metric-chevron" />
    </button>
  );
}

function InventoryRow({
  icon: Icon,
  iconSrc,
  name,
  description,
  meta,
  status,
  tone,
  path,
  official = false,
  deleting = false,
  showStatus = true,
  onOpen,
  onDelete,
}: {
  icon: typeof Box;
  iconSrc?: string | null;
  name: string;
  description: string;
  meta: string;
  status: string;
  tone: StatusTone;
  path: string | null;
  official?: boolean;
  deleting?: boolean;
  showStatus?: boolean;
  onOpen?: () => void;
  onDelete?: () => void;
}) {
  return (
    <article className="inventory-row">
      <div className={iconSrc ? "inventory-icon has-image" : "inventory-icon"}>
        {iconSrc ? <img src={iconSrc} alt="" aria-hidden="true" /> : <Icon size={19} />}
      </div>
      <div className="inventory-copy">
        <div className="inventory-name">
          <strong>{name}</strong>
          {official ? <span className="official-badge" title="OpenAI 官方提供"><ShieldCheck size={12} />官方</span> : null}
        </div>
        <span>{description}</span>
      </div>
      <span className="inventory-meta">{meta}</span>
      {showStatus && status ? <StatusPill tone={tone}>{status}</StatusPill> : null}
      <div className="row-actions">
        {onOpen ? <button className="row-action" type="button" title="查看 Skill 内容" aria-label={`查看 ${name} 内容`} disabled={deleting} onClick={onOpen}><FileText size={17} /></button> : null}
        <button className="row-action" type="button" title="打开所在位置" aria-label={`打开 ${name} 所在位置`} disabled={!path || deleting} onClick={() => path && void revealPath(path)}><FolderOpen size={17} /></button>
        {onDelete ? (
          <button className="row-action delete-action" type="button" title="删除" aria-label={`删除 ${name}`} disabled={deleting} onClick={onDelete}>
            {deleting ? <LoaderCircle size={17} className="spin" /> : <Trash2 size={17} />}
          </button>
        ) : null}
      </div>
    </article>
  );
}

function SkillDetailDialog({
  skill,
  content,
  loading,
  error,
  copied,
  onCopy,
  onClose,
}: {
  skill: SkillItem;
  content: string;
  loading: boolean;
  error: string | null;
  copied: boolean;
  onCopy: () => void;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div className="dialog-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="dialog skill-dialog" role="dialog" aria-modal="true" aria-labelledby="skill-dialog-title">
        <div className="dialog-header">
          <div className="skill-dialog-title">
            <div className={skill.icon ? "inventory-icon has-image" : "inventory-icon"}>
              {skill.icon ? <img src={skill.icon} alt="" aria-hidden="true" /> : <Sparkles size={18} />}
            </div>
            <div><h2 id="skill-dialog-title">{skill.name}</h2><span>SKILL.md</span></div>
          </div>
          <button className="icon-button" type="button" title="关闭" aria-label="关闭" onClick={onClose}><X size={17} /></button>
        </div>
        {loading ? <div className="skill-dialog-state"><LoaderCircle size={22} className="spin" /><span>正在读取 Skill 内容</span></div> : null}
        {error ? <div className="skill-dialog-error"><CircleAlert size={16} /><span>{error}</span></div> : null}
        {!loading && !error ? <MarkdownPreview content={content} /> : null}
        <div className="dialog-actions">
          <button className="secondary-command" type="button" onClick={onClose}>关闭</button>
          <button className="primary-command" type="button" onClick={onCopy} disabled={loading || Boolean(error) || !content}>
            <Clipboard size={16} /> {copied ? "已复制" : "复制全文"}
          </button>
        </div>
      </section>
    </div>
  );
}

function MarkdownPreview({ content }: { content: string }) {
  const lines = content.replaceAll("\r\n", "\n").split("\n");
  const blocks: Array<{ type: "heading" | "paragraph" | "list" | "code" | "rule"; level?: number; text?: string; items?: string[]; language?: string }> = [];
  let index = 0;
  while (index < lines.length) {
    const line = lines[index];
    if (!line.trim()) { index += 1; continue; }
    const fence = line.match(/^\s*```\s*(.*)$/);
    if (fence) {
      const code: string[] = [];
      index += 1;
      while (index < lines.length && !/^\s*```/.test(lines[index])) code.push(lines[index++]);
      if (index < lines.length) index += 1;
      blocks.push({ type: "code", text: code.join("\n"), language: fence[1] || undefined });
      continue;
    }
    const heading = line.match(/^\s*(#{1,6})\s+(.+?)\s*#*\s*$/);
    if (heading) { blocks.push({ type: "heading", level: heading[1].length, text: heading[2] }); index += 1; continue; }
    if (/^\s*(---+|\*\*\*+)\s*$/.test(line)) { blocks.push({ type: "rule" }); index += 1; continue; }
    const list = line.match(/^\s*(?:[-+*]|\d+\.)\s+(.+)$/);
    if (list) {
      const items: string[] = [];
      while (index < lines.length) {
        const item = lines[index].match(/^\s*(?:[-+*]|\d+\.)\s+(.+)$/);
        if (!item) break;
        items.push(item[1]); index += 1;
      }
      blocks.push({ type: "list", items });
      continue;
    }
    const paragraph: string[] = [line.trim()];
    index += 1;
    while (index < lines.length && lines[index].trim() && !/^\s*(?:#{1,6}\s|```|[-+*]\s+|\d+\.\s+)/.test(lines[index])) paragraph.push(lines[index++].trim());
    blocks.push({ type: "paragraph", text: paragraph.join(" ") });
  }

  return <div className="skill-markdown">{blocks.map((block, blockIndex) => {
    if (block.type === "heading") return <MarkdownHeading key={blockIndex} level={block.level ?? 2} text={block.text ?? ""} />;
    if (block.type === "code") return <pre key={blockIndex} className="markdown-code"><code data-language={block.language}>{block.text}</code></pre>;
    if (block.type === "list") return <ul key={blockIndex}>{block.items?.map((item, itemIndex) => <li key={itemIndex}>{renderMarkdownInline(item)}</li>)}</ul>;
    if (block.type === "rule") return <hr key={blockIndex} />;
    return <p key={blockIndex}>{renderMarkdownInline(block.text ?? "")}</p>;
  })}</div>;
}

function MarkdownHeading({ level, text }: { level: number; text: string }) {
  const content = renderMarkdownInline(text);
  if (level === 1) return <h1>{content}</h1>;
  if (level === 2) return <h2>{content}</h2>;
  if (level === 3) return <h3>{content}</h3>;
  if (level === 4) return <h4>{content}</h4>;
  if (level === 5) return <h5>{content}</h5>;
  return <h6>{content}</h6>;
}

function renderMarkdownInline(value: string): ReactNode[] {
  const parts = value.split(/(\*\*[^*]+\*\*|__[^_]+__|`[^`]+`|\[[^\]]+\]\(https?:\/\/[^)]+\))/g).filter(Boolean);
  return parts.map((part, index) => {
    if ((part.startsWith("**") && part.endsWith("**")) || (part.startsWith("__") && part.endsWith("__"))) return <strong key={index}>{part.slice(2, -2)}</strong>;
    if (part.startsWith("`") && part.endsWith("`")) return <code key={index}>{part.slice(1, -1)}</code>;
    const link = part.match(/^\[([^\]]+)\]\((https?:\/\/[^)]+)\)$/);
    if (link) return <a key={index} href={link[2]} target="_blank" rel="noreferrer">{link[1]}</a>;
    return <Fragment key={index}>{part}</Fragment>;
  });
}

function InventoryError({ message }: { message: string }) {
  return <div className="inventory-error" role="alert"><CircleAlert size={16} /><span>{message}</span></div>;
}

function StatusPill({ tone, children }: { tone: StatusTone; children: ReactNode }) {
  return <span className={`status-pill ${tone}`}>{tone === "success" ? <Check size={13} /> : null}{children}</span>;
}

function EmptyState({ title, compact = false }: { title: string; compact?: boolean }) {
  return <div className={compact ? "empty-state compact" : "empty-state"}><Layers3 size={compact ? 20 : 26} /><span>{title}</span></div>;
}

function LoadingState() {
  return <div className="loading-state"><LoaderCircle size={24} className="spin" /><span>正在读取本机 Codex 环境</span></div>;
}

function ErrorBanner({ message, onRetry }: { message: string; onRetry: () => void }) {
  return <div className="error-banner"><CircleAlert size={18} /><span>{message}</span><button type="button" onClick={onRetry}>重试</button></div>;
}

function errorMessage(reason: unknown): string {
  if (reason instanceof Error) return reason.message;
  return String(reason || "发生未知错误");
}

function formatTime(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "刚刚" : date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
}

export default App;
