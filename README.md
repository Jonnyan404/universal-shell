# universal-shell

A configuration-driven manager that downloads, configures, and runs third-party CLI programs, with both **egui** and **Tauri** GUIs sharing one core library (`shared`).

**English** · [中文](#中文文档)

- Downloads/updates program binaries from GitHub into its own data dir (never overwrites itself), verifies SHA256, spawns/stops processes with templated args.
- **Config-driven UI**: the `fields` in `shell.json` decide which widgets render (string / file / directory / boolean / autostart).
- Multiple programs in tabs, each with its own data dir.
- Tray resident: closing the window hides to tray; tray can reopen or quit (both Tauri & egui).

---

## Quick start

### 1. Download

Get the package for your OS from **Releases**.

| Platform | Artifact |
|---|---|
| macOS | `.dmg` |
| Windows | `.msi` or `.exe` |
| Linux | binary |

### 2. Run

A built-in demo program is ready on first launch — just run (egui starts fastest):

```bash
cargo run -p app-egui
```

Or launch the installed app and point it at your own config file:

```bash
cargo run -p app-egui -- /path/to/shell.json
```

### 3. Configure

Load a `shell.json` (see [Config](#config)). Then in the UI:

- **Browse templates** → pick a program → import → it installs & manages itself.
- Or add a program manually via the config file.

### 4. Use

Set the fields, enable autostart if you want, and press **Start**. The program runs in the background and shows under a tray icon.

## Config (`shell.json`)

Field type is set by `fields[].kind`; `args` uses `{key}` to reference field values.

```jsonc
{
  "template_registries": ["https://raw.githubusercontent.com/Jonnyan404/universal-shell/main/registry/"],
  "programs": [
    {
      "id": "cloud-clipboard-go",
      "name": "cloud-clipboard-go",
      "repo": "Jonnyan404/cloud-clipboard-go",
      "binary": "cloud-clipboard-go",
      "assets": {
        "darwin":  { "filename": "{name}_Darwin_{arch}.tar.gz", "format": "tar.gz", "member": "cloud-clipboard-go" },
        "linux":   { "filename": "{name}_Linux_{arch}.tar.gz",  "format": "tar.gz", "member": "cloud-clipboard-go" },
        "windows": { "filename": "{name}_Windows_{arch}.zip",   "format": "zip",     "member": "cloud-clipboard-go.exe" }
      },
      "arch_map": { "x86_64": "x86_64", "aarch64": "arm64" },
      "fields": [
        { "key": "host",     "kind": "string",    "label": "Listen addr", "default": "0.0.0.0" },
        { "key": "port",     "kind": "string",    "label": "Port",        "default": "9000" },
        { "key": "config",   "kind": "file",      "label": "Config file", "required": true },
        { "key": "data_dir", "kind": "directory", "label": "Data dir" },
        { "key": "verbose",  "kind": "boolean",   "label": "Verbose",     "default": false },
        { "key": "autostart","kind": "autostart", "label": "Autostart",   "default": false }
      ],
      "args": ["-host", "{host}", "-port", "{port}", "-config", "{config}", "-dir", "{data_dir}"]
    }
  ]
}
```

Field kinds: `string` / `file` / `directory` / `boolean` / `autostart`. Asset `filename` supports `{name}` `{version}` `{arch}` `{ext}`; `args` supports any `{fieldKey}`. Mark a field `required: true` to block start until filled (its label gets a `*`).

## Data directory

Default: `~/Library/Application Support/universal-shell/` (Windows: `%APPDATA%/universal-shell/`). Each managed program:

- `<data>/<binary>` — the executable
- `<data>/<id>.version` — local version
- `<data>/<id>.values.json` — last form values
- `<data>/logs/<id>.{out,err}.log` — run logs

## Remote template library

- **Registry** publishes `manifests.json` (index) + `templates/<id>.json` (lazy-fetched). Browsing pulls the index only; importing pulls the template.
- **Cache & offline**: both are cached with ETag/If-None-Match; offline shows cached content tagged "offline".
- **Signature (optional)**: set `registry_pubkeys` (base url → Ed25519 public key) to require Ed25519 verification of `manifests.json`.

## Development

Prerequisites: [Rust](https://rustup.rs/) (stable), [Node.js](https://nodejs.org/); macOS Xcode CLT; Linux Tauri deps (`libwebkit2gtk-4.1-dev` etc., see [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#linux)).

```bash
# egui (fastest start)
cargo run -p app-egui

# Tauri
cd app-tauri && npm install && npm run tauri dev

# Useful shared-lib examples
cargo run -p shared --example verify_templates
cargo run -p shared --example generate_registry
cargo run -p shared --example sign_registry

# Package (outputs to dist/, includes sha256.txt)
./scripts/build-release.sh 0.1.0
```

### egui vs Tauri

| | egui (`app-egui`) | Tauri (`app-tauri`) |
|---|---|---|
| Render | native, no WebView | system WebView |
| Size | ~20 MB, single binary | depends on webview |
| Dynamic form | Rust-native render | JS render + backend commands |
| File dialog | rfd | tauri dialog plugin |
| Best for | lightweight tray-resident tool | polished UI / frontend ecosystem |

## Security

- **Trust model**: remote templates download & execute third-party binaries. Defaults to `bind=127.0.0.1` + high port; templates record `template_source`/`imported_at`.
- **Download verification**: template `check_sha256` pin wins; else GitHub Releases `digest` (sha256) is verified byte-by-byte; on failure the file is deleted.
- No telemetry; config & logs stay local.

## License

MIT — see [LICENSE](LICENSE).

---

<a name="中文文档"></a>

# universal-shell（中文）

配置驱动的程序管理壳：在图形界面里下载、配置、启动第三方命令行程序，**egui** 与 **Tauri** 两个前端共享同一个核心库 (`shared`)。

[English](#universal-shell) · 中文文档

- 从 GitHub 自动下载/更新受管程序二进制到独立数据目录（天然防覆盖），SHA256 校验、参数模板替换、启动/停止子进程。
- **配置驱动界面**：`shell.json` 里的 `fields` 决定渲染什么控件（string / file / directory / boolean / autostart）。
- 多程序多 Tab，每个程序独立数据目录。
- 托盘常驻：关窗即隐藏到托盘，托盘可唤出/退出（Tauri 与 egui 双端）。

---

## 快速上手

### 1. 下载

从 **Releases** 下载对应你系统的安装包/二进制。

| 平台 | 产物 |
|---|---|
| macOS | `.dmg` |
| Windows | `.msi` 或 `.exe` |
| Linux | 二进制 |

### 2. 运行

用示例配置直接体验（egui 启动最快）：

```bash
cargo run -p app-egui
```

或启动已安装的应用，指向你自己的配置文件。

### 3. 配置

加载你的 `shell.json`（见下方 [配置格式](#配置格式)）。然后在界面里：

- **浏览模板库** → 选一个程序 → 导入 → 自动安装与管理。
- 或手动在配置文件里添加程序。

### 4. 使用

填好字段、按需勾选开机启动、点 **启动**。程序在后台运行，并出现在托盘图标下。

## 配置格式 (`shell.json`)

字段类型由 `fields[].kind` 决定，`args` 用 `{key}` 引用字段值。

```jsonc
{
  "template_registries": ["https://raw.githubusercontent.com/Jonnyan404/universal-shell/main/registry/"],
  "programs": [
    {
      "id": "cloud-clipboard-go",
      "name": "cloud-clipboard-go",
      "repo": "Jonnyan404/cloud-clipboard-go",
      "binary": "cloud-clipboard-go",
      "assets": {
        "darwin":  { "filename": "{name}_Darwin_{arch}.tar.gz", "format": "tar.gz", "member": "cloud-clipboard-go" },
        "linux":   { "filename": "{name}_Linux_{arch}.tar.gz",  "format": "tar.gz", "member": "cloud-clipboard-go" },
        "windows": { "filename": "{name}_Windows_{arch}.zip",   "format": "zip",     "member": "cloud-clipboard-go.exe" }
      },
      "arch_map": { "x86_64": "x86_64", "aarch64": "arm64" },
      "fields": [
        { "key": "host",     "kind": "string",    "label": "监听地址", "default": "0.0.0.0" },
        { "key": "port",     "kind": "string",    "label": "端口",     "default": "9000" },
        { "key": "config",   "kind": "file",      "label": "配置文件", "required": true },
        { "key": "data_dir", "kind": "directory", "label": "数据目录" },
        { "key": "verbose",  "kind": "boolean",   "label": "详细日志", "default": false },
        { "key": "autostart","kind": "autostart", "label": "开机启动", "default": false }
      ],
      "args": ["-host", "{host}", "-port", "{port}", "-config", "{config}", "-dir", "{data_dir}"]
    }
  ]
}
```

`fields[].kind` 支持：`string` / `file` / `directory` / `boolean` / `autostart`。资产 `filename` 支持 `{name}` `{version}` `{arch}` `{ext}`；`args` 支持任意 `{fieldKey}`。字段标 `"required": true` 后，值为空时无法启动（标签显示 `*`）。

## 数据目录

默认 `~/Library/Application Support/universal-shell/`（Windows: `%APPDATA%/universal-shell/`）。每个受管程序：

- `<data>/<binary>` — 可执行文件
- `<data>/<id>.version` — 本地版本号
- `<data>/<id>.values.json` — 上次表单值
- `<data>/logs/<id>.{out,err}.log` — 运行日志

## 远程模板库

- **两级结构**：`manifests.json`（索引）+ `templates/<id>.json`（惰性拉取）。浏览只拉清单，导入才拉模板。
- **缓存与离线**：清单与模板带缓存 + ETag/If-None-Match 增量校验，断网时显示缓存并标注「离线」。
- **签名（可选）**：配置 `registry_pubkeys` 可强制对 `manifests.json` 做 Ed25519 验签。

## 开发

前置依赖：[Rust](https://rustup.rs/)（stable）、[Node.js](https://nodejs.org/)；macOS 需 Xcode CLT；Linux 需 Tauri 依赖（`libwebkit2gtk-4.1-dev` 等，见 [Tauri 前置](https://v2.tauri.app/start/prerequisites/#linux)）。

```bash
# egui（启动最快）
cargo run -p app-egui

# Tauri
cd app-tauri && npm install && npm run tauri dev

# shared 库常用示例
cargo run -p shared --example verify_templates
cargo run -p shared --example generate_registry
cargo run -p shared --example sign_registry

# 打包（输出到 dist/，含 sha256.txt）
./scripts/build-release.sh 0.1.0
```

### egui 与 Tauri 对比

| | egui (`app-egui`) | Tauri (`app-tauri`) |
|---|---|---|
| 渲染 | 原生绘制，无 WebView | 系统 WebView |
| 体积 | ~20 MB，单二进制 | 依赖 webview |
| 动态表单 | 纯代码遍历渲染 | JS 渲染 + 后端命令 |
| 文件选择 | rfd | tauri dialog 插件 |
| 适合 | 轻量托盘常驻工具 | 界面精致 / 前端生态 |

## 安全说明

- **信任模型**：远程模板会引导下载并执行第三方二进制。默认 `bind=127.0.0.1` + 高位端口；模板记录 `template_source`/`imported_at`。
- **下载校验**：优先模板 `check_sha256`，否则用 GitHub Releases `digest`（sha256）逐字节校验，失败即删除并报错。
- 不收集任何遥测，配置与日志只写本地。

## 许可

MIT — 详见 [LICENSE](LICENSE)。
