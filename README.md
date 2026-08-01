# codex_go

`codex_go` 是面向 Windows 用户的 Codex 本机管理器。它检测 Microsoft Store 分发的官方 Codex Windows 桌面端，展示当前用户 `CODEX_HOME` 中已安装的插件和 Skills，并可安装、更新或从 Codex Go 直接启动官方桌面端。

这是一个真正的 Tauri Windows 桌面程序，不需要用浏览器打开。浏览器里的 `127.0.0.1:3000` 只是前端开发预览，不具备完整的本机功能。

## 在 Windows 运行

普通用户应下载 GitHub Releases 中的 `codex_go_*_x64-setup.exe` 并双击安装。安装后从开始菜单打开 `codex_go`。

如果当前正在 WSL 中开发，请从 Windows 资源管理器打开：

```text
\\wsl.localhost\<你的发行版>\home\codex_go
```

第一次开发需要在 Windows PowerShell 执行：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\setup-windows-dev.ps1
```

重新打开一个 Windows 终端后，运行桌面开发版：

```cmd
scripts\run-windows.cmd
```

生成本机 NSIS 安装包：

```cmd
scripts\build-windows.cmd
```

两个批处理都使用 `pushd`，因此也能从 `\\wsl.localhost\...` UNC 路径执行。安装包输出到 `src-tauri\target\release\bundle\nsis\`。

## 当前功能

- 通过 `Get-AppxPackage OpenAI.Codex` 检测官方 Codex Windows 桌面端，并读取 Microsoft Store 包版本与安装位置。
- “运行”会通过 Windows 打包应用激活器从 Codex Go 启动官方桌面端，再通过仅监听本机的 Chromium DevTools Protocol 注入右上角 `Codex Go <版本号>` 标识，不修改官方应用文件。
- 运行状态明确区分“未运行”“未通过 Codex Go 运行”“已运行”。已有可见的 Codex 窗口不是由 Codex Go 启动时不会强行接管；只有没有窗口的残留后台进程会在用户点击“运行”后自动清理，以免阻塞启动。
- 扫描 `CODEX_HOME` 中的插件 manifest 和缓存。
- 扫描个人、系统及插件附带的 `SKILL.md`，限制递归深度且不跟随目录链接。
- 使用固定版本 Xray，将内置网络通道转为带随机认证的本机 HTTP 代理。
- Codex Windows 桌面端使用 Microsoft Store 官方渠道安装和更新；需要 Windows App Installer（`winget`）。首次通过内置 VLESS 执行 Store 任务时，应用会请求一次管理员授权以启用 WinGet 代理参数，无需用户手动运行设置命令。
- 代理只监听 `127.0.0.1`，覆盖 `codex_go` 发起的软件更新以及 Codex 安装、升级任务，不修改 Windows 系统代理。
- 启动后读取公开 Release 的 `latest.json`，比较当前版本，并在用户确认后下载、验签和安装更新。
- 网络通道固定内置，没有设置或覆盖入口，也不会在 UI 或日志中回显链接。

## 本地验证

前置条件为 Windows 10/11 x64、Node.js 20.19+ 或 22.12+、Rust stable、Visual Studio C++ Build Tools 和 WebView2。可分别运行：

```powershell
npm ci
npm run test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

## 远程更新与发布

项目只使用一个公开的 GitHub 仓库 `jiahaochat/codex_go`，源码、安装包、Tauri 签名和 `latest.json` 都在同一个仓库。这样普通用户无需 Git 或 GitHub Token 即可访问 Release 更新。

该仓库需要以下 Actions Secrets：

- `TAURI_SIGNING_PRIVATE_KEY`：Tauri updater 私钥。
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：私钥密码。

Updater 私钥一旦丢失，就无法再给已安装客户端发布可验证的更新。首次配置 Actions Secret 时，应把加密私钥及其密码另存到受控的离线备份；备份不能提交到 Git。

发布新版本：

1. 同步修改 `package.json`、`src-tauri/Cargo.toml` 和 `src-tauri/tauri.conf.json` 的版本号。
2. 提交并推送 `main`，等待 Windows 构建通过。
3. 创建并推送同版本标签，例如 `git tag -a v0.2.0 -m "codex_go v0.2.0"` 和 `git push origin v0.2.0`。
4. GitHub Actions 只接受 `main` 历史上的标签，按 `Cargo.lock` 构建签名 NSIS 包，并在同一仓库的草稿 Release 中生成和验证 `latest.json`。
5. 元数据验证通过后工作流才公开 Release。客户端通过内置网络通道访问该文件，按语义版本提示更新。

发布工作流会把 updater 元数据中的 GitHub API asset 地址改写为同一仓库的公开 Release 直链，避免共享出口带来的匿名 API 频率限制。

更新器使用独立的 Tauri/minisign 签名验证包内容。代码签名（Authenticode）是另一层机制，正式广泛分发前仍建议购买证书以减少 Windows SmartScreen 提示。

## 固定资源

GitHub Actions 和本地打包脚本会下载并校验：

- Xray-core `v26.3.27`

## 安全边界

桌面程序中的固定网络凭据无法真正保密：发布后的 EXE 和运行内存都可以被分析。后续若用户规模扩大，应改为服务端签发单设备、可撤销、有限额的凭据。

网络凭据和 GitHub Token 都不应出现在 Actions 日志或 issue。已经公开发送过的凭据应及时轮换。

## 数据读取范围

Drive 登录后，应用管理 `\\drive\cloud\<当前用户>\.codex` 中的插件、Skills 和 manifest。从 Codex Go 启动官方 Codex 时，同一路径仅通过该 Codex 子进程的 `CODEX_HOME` 环境变量传入；目录不存在时由 Codex 自行创建。通过其他入口启动 Codex 不受影响，仍使用其默认路径。应用不会读取或展示 `auth.json`、会话正文或 API Key。

## 许可

`codex_go` 使用 MIT License。随包分发的 Xray-core 是独立 MPL-2.0 程序，完整说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
