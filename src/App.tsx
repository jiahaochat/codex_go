import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Box,
  Check,
  ChevronRight,
  CircleAlert,
  CloudDownload,
  Copy,
  Download,
  FolderOpen,
  Gauge,
  Layers3,
  LoaderCircle,
  Minus,
  PackageCheck,
  Plug,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
  Square,
  Sparkles,
  TerminalSquare,
  X,
} from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import codexGoIcon from "../assets/codex-go-icon.png";
import {
  checkAppUpdate,
  getSnapshot,
  installAppUpdate,
  installCodex,
  updateCodex,
  onAppUpdateProgress,
  onInstallProgress,
  refreshSnapshot,
  revealPath,
  type AppSnapshot,
  type InstallProgress,
  type PluginItem,
  type SkillItem,
  type UpdateInfo,
  type UpdateProgress,
} from "./api";
import {
  filterPlugins,
  filterSkills,
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

function App() {
  const [view, setView] = useState<View>("overview");
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<InstallProgress | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);
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

  const startCodexAction = async () => {
    if (updating) {
      setInstallError("请等待软件更新任务结束");
      return;
    }
    setInstallError(null);
    setInstalling(true);
    const isUpdate = snapshot?.codex.installed;
    setProgress({ stage: "preparing", percent: 4, message: isUpdate ? "正在检查 Codex Windows 更新" : "正在准备 Codex Windows 安装" });
    try {
      if (isUpdate) await updateCodex();
      else await installCodex();
    } catch (reason) {
      const message = errorMessage(reason);
      setInstalling(false);
      setInstallError(message);
      setProgress({ stage: "error", percent: 100, message });
    }
  };

  const startAppUpdate = async () => {
    if (installing) {
      setUpdateError("请等待 Codex 安装任务结束");
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
                blocked={installing}
                progress={updateProgress}
                onInstall={() => void startAppUpdate()}
                onDismiss={() => setUpdateDismissed(true)}
              />
            ) : null}
            {installError ? <ErrorBanner message={installError} onRetry={() => void startCodexAction()} /> : null}
            {updateError ? <ErrorBanner message={updateError} onRetry={() => void checkUpdates(true)} /> : null}
            {snapshot && view === "overview" ? (
              <Overview
                snapshot={snapshot}
                installing={installing}
                operationBusy={installing || updating}
                progress={progress}
                onInstall={() => void startCodexAction()}
                onNavigate={setView}
              />
            ) : null}
            {snapshot && view === "plugins" ? <PluginInventory items={snapshot.plugins} /> : null}
            {snapshot && view === "skills" ? <SkillInventory items={snapshot.skills} /> : null}
            {snapshot && view === "settings" ? (
              <SettingsView
                snapshot={snapshot}
                installing={installing}
                operationBusy={installing || updating}
                progress={progress}
                onInstall={() => void startCodexAction()}
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
        <span className={snapshot?.codex.installed ? "status-dot online" : "status-dot"} />
        <div>
          <strong>{snapshot?.codex.installed ? "Codex 已连接" : "等待 Codex"}</strong>
          <span>{snapshot?.codex.version ?? "尚未检测到版本"}</span>
        </div>
      </div>
    </aside>
  );
}

function Overview({
  snapshot,
  installing,
  operationBusy,
  progress,
  onInstall,
  onNavigate,
}: {
  snapshot: AppSnapshot;
  installing: boolean;
  operationBusy: boolean;
  progress: InstallProgress | null;
  onInstall: () => void;
  onNavigate: (view: View) => void;
}) {
  return (
    <div className="view-stack">
      <CodexHero
        snapshot={snapshot}
        installing={installing}
        operationBusy={operationBusy}
        progress={progress}
        onInstall={onInstall}
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
              name={plugin.name}
              description={plugin.description ?? plugin.marketplace ?? "Codex 插件"}
              meta={plugin.version ? `v${plugin.version}` : sourceLabels[plugin.source]}
              status={plugin.enabled ? "已启用" : "未启用"}
              tone={plugin.enabled ? "success" : "neutral"}
              path={plugin.path}
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
  installing,
  operationBusy,
  progress,
  onInstall,
}: {
  snapshot: AppSnapshot;
  installing: boolean;
  operationBusy: boolean;
  progress: InstallProgress | null;
  onInstall: () => void;
}) {
  const installed = snapshot.codex.installed;
  return (
    <section className={installed ? "codex-hero installed" : "codex-hero missing"}>
      <div className="hero-icon">
        {installed ? <TerminalSquare size={29} /> : <Download size={29} />}
      </div>
      <div className="hero-copy">
        <div className="hero-title-line">
          <h2>{installed ? "Codex Windows 桌面端已安装" : "尚未安装 Codex Windows 桌面端"}</h2>
          <StatusPill tone={installed ? "success" : "warning"}>
            {installed ? snapshot.codex.version ?? "版本未知" : "需要安装"}
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
        {installing && progress ? (
          <div className="install-progress" aria-live="polite">
            <div className="progress-label"><span>{progress.message}</span><b>{progress.percent}%</b></div>
            <div className="progress-track"><i style={{ width: `${progress.percent}%` }} /></div>
          </div>
        ) : null}
      </div>
      <div className="hero-action">
        <button className="primary-command" type="button" onClick={onInstall} disabled={operationBusy}>
          {installing ? <LoaderCircle size={17} className="spin" /> : <Download size={17} />}
          {installing ? (installed ? "正在更新" : "正在安装") : (installed ? "更新到最新版" : "安装 Codex")}
        </button>
      </div>
    </section>
  );
}

function PluginInventory({ items }: { items: PluginItem[] }) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<PluginFilter>("all");
  const visible = useMemo(() => filterPlugins(items, query, filter), [filter, items, query]);

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
      {visible.map((plugin) => (
        <InventoryRow
          key={plugin.id}
          icon={PackageCheck}
          name={plugin.name}
          description={plugin.description ?? `${plugin.marketplace ?? "本地"} 插件`}
          meta={[plugin.version ? `v${plugin.version}` : null, sourceLabels[plugin.source]].filter(Boolean).join(" · ")}
          status={plugin.enabled ? "已启用" : "未启用"}
          tone={plugin.enabled ? "success" : "neutral"}
          path={plugin.path}
        />
      ))}
      {!visible.length ? <EmptyState title={items.length ? "没有符合条件的插件" : "尚未识别到插件"} /> : null}
    </InventoryView>
  );
}

function SkillInventory({ items }: { items: SkillItem[] }) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<SkillFilter>("all");
  const visible = useMemo(() => filterSkills(items, query, filter), [filter, items, query]);

  return (
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
      {visible.map((skill) => (
        <InventoryRow
          key={skill.id}
          icon={Sparkles}
          name={skill.name}
          description={skill.description ?? "暂无描述"}
          meta={[originLabels[skill.origin], skill.pluginName, sourceLabels[skill.source]].filter(Boolean).join(" · ")}
          status={originLabels[skill.origin]}
          tone={skill.origin === "personal" ? "success" : "neutral"}
          path={skill.path}
        />
      ))}
      {!visible.length ? <EmptyState title={items.length ? "没有符合条件的 Skill" : "尚未识别到 Skill"} /> : null}
    </InventoryView>
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
  children: React.ReactNode;
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
  installing,
  operationBusy,
  progress,
  onInstall,
  update,
  updateProgress,
  checkingUpdate,
  updateChecked,
  updating,
  onCheckUpdate,
  onInstallUpdate,
}: {
  snapshot: AppSnapshot;
  installing: boolean;
  operationBusy: boolean;
  progress: InstallProgress | null;
  onInstall: () => void;
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
          <div><dt>版本</dt><dd>{snapshot.codex.version ?? "-"}</dd></div>
          <div><dt>安装位置</dt><dd className="path-value">{snapshot.codex.path ?? "未检测到"}</dd></div>
          <div><dt>分发渠道</dt><dd>{snapshot.codex.source ?? "-"}</dd></div>
          <div><dt>CODEX_HOME</dt><dd className="path-value">{snapshot.codexHome}</dd></div>
        </dl>
        <button className="primary-command" type="button" onClick={onInstall} disabled={operationBusy}>
          {installing ? <LoaderCircle size={17} className="spin" /> : <Download size={17} />}
          {installing ? (snapshot.codex.installed ? progress?.message ?? "正在更新" : progress?.message ?? "正在安装") : snapshot.codex.installed ? "检查并更新到最新版" : "安装官方 Codex Windows 桌面端"}
        </button>
        {!snapshot.codex.installed ? null : (
          <button className="secondary-command settings-open-path" type="button" onClick={() => snapshot.codex.path && void revealPath(snapshot.codex.path)} disabled={!snapshot.codex.path}>
            <FolderOpen size={17} /> 打开安装位置
          </button>
        )}
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

function InventoryRow({ icon: Icon, name, description, meta, status, tone, path }: { icon: typeof Box; name: string; description: string; meta: string; status: string; tone: StatusTone; path: string | null }) {
  return (
    <article className="inventory-row">
      <div className="inventory-icon"><Icon size={19} /></div>
      <div className="inventory-copy"><strong>{name}</strong><span>{description}</span></div>
      <span className="inventory-meta">{meta}</span>
      <StatusPill tone={tone}>{status}</StatusPill>
      <button className="row-action" type="button" title="打开所在位置" aria-label={`打开 ${name} 所在位置`} disabled={!path} onClick={() => path && void revealPath(path)}><FolderOpen size={17} /></button>
    </article>
  );
}

function StatusPill({ tone, children }: { tone: StatusTone; children: React.ReactNode }) {
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
