# codex_go

`codex_go` 是面向国内 Windows 用户的 Codex 本机管理器。它检测 Codex CLI，展示当前用户 `CODEX_HOME` 中已安装的插件和 Skills，并在 Codex 缺失时通过内置的 VLESS/Xray 线路运行 OpenAI 官方安装程序。

这是一个真正的 Tauri Windows 桌面程序，不需要用浏览器打开。浏览器里的 `127.0.0.1:1420` 只是前端开发预览，不具备完整的本机功能。

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

- 从 `PATH`、OpenAI standalone 默认目录和 npm 全局目录检测 `codex`，并读取版本。
- 优先通过 `codex plugin list --json` 获取插件状态，失败时扫描 `CODEX_HOME` 中的 manifest 和缓存。
- 扫描个人、系统及插件附带的 `SKILL.md`，限制递归深度且不跟随目录链接。
- 使用固定版本 Xray，将 VLESS + REALITY + XHTTP 转为带随机认证的本机 HTTP 代理。
- 代理只监听 `127.0.0.1`，只覆盖 Codex 安装、应用更新检查和更新包下载，不修改 Windows 系统代理。
- 使用固定 OpenAI Codex 提交中的官方 `install.ps1`，运行前再次校验 SHA-256。
- 启动后读取公开 Release 的 `latest.json`，比较当前版本，并在用户确认后下载、验签和安装更新。
- VLESS 链路只从构建 Secret 注入；正式版本没有线路设置入口，也不会在 UI 或日志中回显链接。

## 本地验证

前置条件为 Windows 10/11 x64、Node.js 20.19+ 或 22.12+、Rust stable、Visual Studio C++ Build Tools 和 WebView2。可分别运行：

```powershell
npm ci
npm run test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

开发时可在当前 PowerShell 会话临时设置 `CODEX_GO_VLESS_URI`。它只是未注入构建 Secret 时的开发后备，不要把真实值写入项目文件。

## 远程更新与发布

源码仓库 `jiahaochat/codex_go` 保持私有；更新产物发布到公开仓库 `jiahaochat/codex_go-releases`。公开仓库只存安装包、Tauri 签名和 `latest.json`，不公开源码。普通用户无需 Git 或 GitHub Token。

私有源码仓库需要以下 Actions Secrets：

- `CODEX_GO_DEFAULT_VLESS_URI`：内置下载线路。
- `TAURI_SIGNING_PRIVATE_KEY`：Tauri updater 私钥。
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：私钥密码。
- `RELEASE_REPO_TOKEN`：仅用于 Actions 向公开发布仓库创建 Release。

Updater 私钥一旦丢失，就无法再给已安装客户端发布可验证的更新。首次配置 Actions Secret 时，应把加密私钥及其密码另存到受控的离线备份；备份不能提交到 Git。

发布新版本：

1. 同步修改 `package.json`、`src-tauri/Cargo.toml` 和 `src-tauri/tauri.conf.json` 的版本号。
2. 提交并推送 `main`，等待 Windows 构建通过。
3. 创建并推送同版本标签，例如 `git tag -a v0.2.0 -m "codex_go v0.2.0"` 和 `git push origin v0.2.0`。
4. GitHub Actions 构建签名 NSIS 包并在公开仓库生成 `latest.json`。客户端通过内置线路访问该文件，按语义版本提示更新。

发布工作流会把 updater 元数据中的 GitHub API asset 地址改写为公开 Release 直链，避免所有用户共用 VLESS 出口时共享匿名 API 频率限制。

更新器使用独立的 Tauri/minisign 签名验证包内容。代码签名（Authenticode）是另一层机制，正式广泛分发前仍建议购买证书以减少 Windows SmartScreen 提示。

## 固定资源

GitHub Actions 和本地打包脚本会下载并校验：

- Xray-core `v26.3.27`
- OpenAI Codex installer commit `6219b7c40fc9c702c0aef9964e72b492558f60e4`

## 安全边界

桌面程序中的固定 VLESS 链接无法真正保密：发布后的 EXE 和运行内存都可以被用户分析。构建 Secret 能防止链接进入 Git 历史，但不能保护发布包中的共享凭据。后续若用户规模扩大，应改为服务端签发单设备、可撤销、有限额的线路。

线路和 GitHub Token 都不应写入源码、`.env`、Actions 日志或 issue。已经在聊天中发送过的凭据应轮换。

## 数据读取范围

应用读取 `CODEX_HOME`（默认 `%USERPROFILE%\.codex`）中的插件、Skills 和 manifest。它不会读取或展示 `auth.json`、会话正文或 API Key。

## 许可

`codex_go` 使用 MIT License。随包分发的 Xray-core 是独立 MPL-2.0 程序，完整说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
