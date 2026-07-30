import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Box,
  Check,
  ChevronRight,
  CircleAlert,
  CloudDownload,
  Download,
  FolderOpen,
  Gauge,
  Layers3,
  LoaderCircle,
  PackageCheck,
  Plug,
  RefreshCw,
  Route,
  Search,
  Settings2,
  ShieldCheck,
  Sparkles,
  TerminalSquare,
  X,
} from "lucide-react";
import {
  checkAppUpdate,
  getSnapshot,
  installAppUpdate,
  installCodex,
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
  command: "Codex CLI",
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

  const startInstall = async () => {
    if (updating) {
      setInstallError("请等待软件更新任务结束");
      return;
    }
    if (!snapshot?.proxy.configured) {
      setError("当前发布包未内置加速线路，请重新下载安装包");
      return;
    }
    setInstallError(null);
    setInstalling(true);
    setProgress({ stage: "preparing", percent: 4, message: "正在准备安装环境" });
    try {
      await installCodex();
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
    setUpdateProgress({ stage: "proxy", percent: 3, message: "正在准备更新线路" });
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
          {installError ? <ErrorBanner message={installError} onRetry={() => void startInstall()} /> : null}
          {updateError ? <ErrorBanner message={updateError} onRetry={() => void checkUpdates(true)} /> : null}
          {snapshot && view === "overview" ? (
            <Overview
              snapshot={snapshot}
              installing={installing}
              operationBusy={installing || updating}
              progress={progress}
              onInstall={() => void startInstall()}
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
              onInstall={() => void startInstall()}
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
      <div className="brand">
        <div className="brand-mark" aria-hidden="true">
          <span>&gt;</span>
          <i />
        </div>
        <div>
          <strong>codex_go</strong>
          <span>Windows</span>
        </div>
      </div>
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
        <Metric
          icon={Route}
          label="下载线路"
          value={snapshot.proxy.configured ? "已内置" : "构建缺失"}
          detail={snapshot.proxy.coreAvailable ? "Xray 核心可用" : "缺少 Xray 核心"}
          accent="green"
          onClick={() => onNavigate("settings")}
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
          <h2>{installed ? "Codex 已安装" : "尚未安装 Codex"}</h2>
          <StatusPill tone={installed ? "success" : "warning"}>
            {installed ? snapshot.codex.version ?? "版本未知" : "需要安装"}
          </StatusPill>
        </div>
        <p>
          {installed
            ? snapshot.codex.path ?? "已通过系统命令识别 Codex"
            : "使用加速线路获取 OpenAI 官方 Windows 安装包。"}
        </p>
        <div className="hero-meta">
          <span><FolderOpen size={15} /> {snapshot.codexHome}</span>
          <span><ShieldCheck size={15} /> {snapshot.proxy.configured ? "安装线路已配置" : "安装线路待配置"}</span>
        </div>
        {installing && progress ? (
          <div className="install-progress" aria-live="polite">
            <div className="progress-label"><span>{progress.message}</span><b>{progress.percent}%</b></div>
            <div className="progress-track"><i style={{ width: `${progress.percent}%` }} /></div>
          </div>
        ) : null}
      </div>
      <div className="hero-action">
        {installed ? (
          <button
            className="secondary-command"
            type="button"
            onClick={() => snapshot.codex.path && void revealPath(snapshot.codex.path)}
            disabled={!snapshot.codex.path}
          >
            <FolderOpen size={17} /> 打开位置
          </button>
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
          <div><h2>Codex CLI</h2><p>官方独立安装与本机检测</p></div>
        </div>
        <dl className="details-list">
          <div><dt>状态</dt><dd><StatusPill tone={snapshot.codex.installed ? "success" : "warning"}>{snapshot.codex.installed ? "已安装" : "未安装"}</StatusPill></dd></div>
          <div><dt>版本</dt><dd>{snapshot.codex.version ?? "-"}</dd></div>
          <div><dt>命令位置</dt><dd className="path-value">{snapshot.codex.path ?? "未检测到"}</dd></div>
          <div><dt>CODEX_HOME</dt><dd className="path-value">{snapshot.codexHome}</dd></div>
        </dl>
        {!snapshot.codex.installed ? (
          <button className="primary-command" type="button" onClick={onInstall} disabled={operationBusy}>
            {installing ? <LoaderCircle size={17} className="spin" /> : <Download size={17} />}
            {installing ? progress?.message ?? "正在安装" : "安装官方 Codex"}
          </button>
        ) : null}
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
          <div><dt>检查线路</dt><dd>内置 VLESS 加速</dd></div>
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

      <section className="settings-section">
        <div className="settings-heading">
          <div className="settings-icon route"><Route size={20} /></div>
          <div><h2>下载线路</h2><p>仅用于 Codex 安装和更新请求</p></div>
        </div>
        <dl className="details-list">
          <div><dt>VLESS 配置</dt><dd><StatusPill tone={snapshot.proxy.configured ? "success" : "warning"}>{snapshot.proxy.configured ? "已内置" : "构建缺失"}</StatusPill></dd></div>
          <div><dt>Xray 核心</dt><dd>{snapshot.proxy.coreAvailable ? "可用" : "安装包未包含"}</dd></div>
          <div><dt>凭据来源</dt><dd>{proxySourceLabel(snapshot.proxy.source)}</dd></div>
          <div><dt>作用范围</dt><dd>Codex 安装与软件更新</dd></div>
        </dl>
      </section>

      <section className="security-strip">
        <ShieldCheck size={20} />
        <div><strong>进程级代理</strong><span>本机代理仅监听 127.0.0.1，任务结束后自动关闭，不修改 Windows 系统代理。</span></div>
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

function proxySourceLabel(source: AppSnapshot["proxy"]["source"]): string {
  return { environment: "开发环境", build: "内置发布配置", none: "无" }[source];
}

export default App;
