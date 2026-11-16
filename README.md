# universal-shell

A configuration-driven manager that downloads, configures, and runs third-party CLI programs, with a **Tauri** desktop GUI and a headless web manager sharing one core library (`shared`).

**English** · [中文](#中文文档)

- Downloads/updates program binaries from GitHub into its own data dir (never overwrites itself), verifies SHA256, spawns/stops processes with templated args.
- **Config-driven UI**: the `fields` in `shell.json` decide which widgets render (string / file / directory / boolean).
- Multiple programs in tabs, each with its own data dir.
- Tray resident: closing the window hides to tray; tray can reopen or quit.

---

## Quick start

### 0. Pick an interface

| | Tauri (`app-tauri`) | CLI (`us-web`, `app-cli`) |
|---|---|---|
| What it is | Full-featured desktop app (system WebView) | Headless web manager, no window |
| Best for | Polished UI / rich frontend | NAS, Raspberry Pi, headless server |
| Rendering | System WebView | Browser only |
| File picker | tauri dialog plugin | manual / upload via web |

Both share the same config, data directory and template library — safe to switch between them anytime.

> The former **egui** build (`app-egui`) is archived: its source stays in the repo for reference but it is
> no longer built, packaged or shipped. See [ROADMAP](ROADMAP.md#关键决策记录) for the reasoning.

### 1. Download

Get the package for your OS from **Releases**.

| Platform | Artifact |
|---|---|
| macOS | `.dmg` |
| Windows | `.msi` or `-setup.exe` |
| Linux | `.deb` / `.rpm` / `.AppImage` |
| Headless server (any OS) | `universal-shell-cli-*` (the `us-web` binary) |

### 2. First-run workflow (remote template library)

The primary way to get programs is the **template library**: browse → import → run. No need to hand-write config.

1. Open **Template library** (tab in the sidebar).
2. Click **Refresh** to fetch the library.
   > **Network note**: refreshing hits GitHub directly. If GitHub is unreachable in your region or times out, configure the network settings under **Settings** (accelerate prefix and/or proxy) first, then refresh again.
3. **Import** the programs you need — they are installed and managed automatically (SHA256-verified downloads, no manual config).

That's it. Picked templates become instances you can **Start / Stop** right away.

> Alternatively, you can skip the library and manage programs by hand in `shell.json` (see [Config](#config)).

### 3. Use

Set the fields if needed, enable autostart if you want, and press **Start**. The program runs in the background and shows under a tray icon.

## Config (`shell.json`)

Field type is set by `fields[].kind`; `args` uses `{key}` to reference field values.

```jsonc
{
  "template_registries": ["https://raw.githubusercontent.com/Jonnyan404/universal-shell-templates/main/registry/"],
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
        { "key": "verbose",  "kind": "boolean",   "label": "Verbose",     "default": false }
      ],
      "args": ["-host", "{host}", "-port", "{port}", "-config", "{config}", "-dir", "{data_dir}"]
    }
  ]
}
```

Field kinds: `string` / `file` / `directory` / `boolean`. Asset `filename` supports `{name}` `{version}` `{arch}` `{ext}`; `args` supports any `{fieldKey}`. Mark a field `required: true` to block start until filled (its label gets a `*`). Program autostart is shell-managed state (the shell's autostart toggle/`program-autostart.json`), not a template field.

## Data directory

Default: `~/Library/Application Support/universal-shell/` (Windows: `%APPDATA%/universal-shell/`). Each managed program:

- `<data>/<binary>` — the executable
- `<data>/<id>.version` — local version
- `<data>/<id>.values.json` — last form values
- `<data>/logs/<id>.{out,err}.log` — run logs

## Remote template library

- **Official library lives in its own repo**: [universal-shell-templates](https://github.com/Jonnyan404/universal-shell-templates) holds `templates/*.json` (source of truth) plus the generated, signed `registry/` served over raw. That repo is deliberately code-free (no tooling, no CI): regenerate `registry/` from this repo's tooling (`generate_registry` + `sign_registry`) and commit it together with the template change; verify with `verify_templates` / `e2e_download` pointed at a templates checkout.
- **Registry** publishes `manifests.json` (index) + `templates/<id>.json` (lazy-fetched). Browsing pulls the index only; importing pulls the template.
- **Cache & offline**: both are cached with ETag/If-None-Match; offline shows cached content tagged "offline".
- **Signature (optional)**: set `registry_pubkeys` (base url → Ed25519 public key) to require Ed25519 verification of `manifests.json`.

## Development

Prerequisites: [Rust](https://rustup.rs/) (stable), [Node.js](https://nodejs.org/); macOS Xcode CLT; Linux Tauri deps (`libwebkit2gtk-4.1-dev` etc., see [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#linux)).

```bash
# Tauri (desktop app + embedded web manager)
cd app-tauri && npm install && npm run tauri dev

# Headless web manager only (no window)
cargo run -p app-cli -- --port 0

# Useful shared-lib examples (the template library lives in its own repo — clone
# universal-shell-templates next to this one, or point the env vars at a checkout)
cargo run -p shared --example verify_templates -- ../universal-shell-templates/templates
cargo run -p shared --example generate_registry -- ../universal-shell-templates/registry ../universal-shell-templates/templates
cargo run -p shared --example sign_registry -- ../universal-shell-templates/registry/manifests.json
cargo run -p shared --example registry_demo -- ../universal-shell-templates/registry

# Package (outputs to dist/, includes sha256.txt)
./scripts/build-release.sh 0.1.0
```

### Archived egui build

`app-egui/` still lives in this repo, but it is **excluded from the workspace** (`Cargo.toml: exclude`),
from `scripts/build-release.sh`, and from every CI workflow. It is not built, not packaged, and not shipped.

Reasons (see [ROADMAP — 决策记录](ROADMAP.md#关键决策记录)):

- The Tauri window and the headless `us-web` server already cover its two use cases; keeping a third
  app meant implementing every new feature 2–3 times while `app-egui` gradually fell behind
  (no log center, no template diff, no binary upload, no LAN toggle, no theme switch; 258 of 486 i18n keys).
- Release assets showed no meaningful downloads, so the duplicated maintenance served no users.

To revive it: drop the empty `[workspace]` table and the inlined `version`/`edition`/`license`/`authors`
from `app-egui/Cargo.toml` (restore `workspace = true` inheritance), remove `exclude`, add it back to
`members`, then restore the packaging steps in `scripts/build-release.sh` and the CI workflows.

## Security

- **Trust model**: remote templates download & execute third-party binaries. Defaults to `bind=127.0.0.1` + high port; templates record `template_source`/`imported_at`.
- **Download verification**: template `check_sha256` pin wins; else GitHub Releases `digest` (sha256) is verified byte-by-byte; on failure the file is deleted.
- No telemetry; config & logs stay local.

## License

MIT — see [LICENSE](LICENSE).

---

<a name="中文文档"></a>

# universal-shell（中文）

配置驱动的程序管理壳：在图形界面里下载、配置、启动第三方命令行程序，**Tauri** 桌面界面与无窗口 Web 管理服务共享同一个核心库 (`shared`)。

[English](#universal-shell) · 中文文档

- 从 GitHub 自动下载/更新受管程序二进制到独立数据目录（天然防覆盖），SHA256 校验、参数模板替换、启动/停止子进程。
- **配置驱动界面**：`shell.json` 里的 `fields` 决定渲染什么控件（string / file / directory / boolean）。
- 多程序多 Tab，每个程序独立数据目录。
- 托盘常驻：关窗即隐藏到托盘，托盘可唤出/退出。

---

## 快速上手

### 0. 先选界面

| | Tauri（`app-tauri`） | CLI（`us-web`，`app-cli`） |
|---|---|---|
| 是什么 | 完整桌面应用（系统 WebView） | 无窗口 Web 管理服务 |
| 适合 | 界面精致 / 前端生态 | 树莓派 / NAS / 无桌面服务器 |
| 渲染 | 系统 WebView | 仅浏览器 |
| 文件选择 | tauri dialog 插件 | 手填 / 网页上传 |

两者共用同一份配置、数据目录和模板库，可随时切换。

> 原 **egui** 版（`app-egui`）已归档：源码留在仓库里备查，但不再构建、打包或分发。
> 原因见 [ROADMAP 决策记录](ROADMAP.md#关键决策记录)。

### 1. 下载

从 **Releases** 下载对应你系统的安装包/二进制。

| 平台 | 产物 |
|---|---|
| macOS | `.dmg` |
| Windows | `.msi` 或 `-setup.exe` |
| Linux | `.deb` / `.rpm` / `.AppImage` |
| 无桌面服务器（任意系统） | `universal-shell-cli-*`（即 `us-web` 二进制） |

### 2. 首次使用（远程模板库流程）

获取程序的主要方式是**模板库**：浏览 → 导入 → 运行，全程无需手写配置。

1. 打开侧栏的「**模板库**」页。
2. 点击「**刷新**」拉取模板库。
   > **网络提示**：刷新会直连 GitHub。如果你的网络访问 GitHub 不通或超时，先去「设置」里配置网络代理（加速地址和/或通用代理），再回来刷新。
3. **导入**你需要的程序——自动下载安装并纳入管理（SHA256 校验，无需手动配置）。

完成。导入的模板会成为可立即「**启动 / 停止**」的实例。

> 也可以不走模板库，直接在 `shell.json` 里手写程序（见下方 [配置格式](#配置格式-shelljson)）。

### 3. 使用

需要时填好字段、按需勾选开机启动，点 **启动**。程序在后台运行，并出现在托盘图标下。

## 配置格式 (`shell.json`)

字段类型由 `fields[].kind` 决定，`args` 用 `{key}` 引用字段值。

```jsonc
{
  "template_registries": ["https://raw.githubusercontent.com/Jonnyan404/universal-shell-templates/main/registry/"],
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
        { "key": "verbose",  "kind": "boolean",   "label": "详细日志", "default": false }
      ],
      "args": ["-host", "{host}", "-port", "{port}", "-config", "{config}", "-dir", "{data_dir}"]
    }
  ]
}
```

`fields[].kind` 支持：`string` / `file` / `directory` / `boolean`。资产 `filename` 支持 `{name}` `{version}` `{arch}` `{ext}`；`args` 支持任意 `{fieldKey}`。字段标 `"required": true` 后，值为空时无法启动（标签显示 `*`）。程序自启动是壳统一管理的状态（自启开关 / `program-autostart.json`），不是模板字段。

## 数据目录

默认 `~/Library/Application Support/universal-shell/`（macOS）、`~/.local/share/universal-shell/`（Linux，遵循 `$XDG_DATA_HOME`）、`%APPDATA%/universal-shell/`（Windows）。每个受管程序：

- `<data>/<binary>` — 可执行文件
- `<data>/<id>.version` — 本地版本号
- `<data>/<id>.values.json` — 上次表单值
- `<data>/logs/<id>.{out,err}.log` — 运行日志

## 远程模板库

- **官方模板库已拆为独立仓库**：[universal-shell-templates](https://github.com/Jonnyan404/universal-shell-templates) 放 `templates/*.json`（单一事实来源）与生成并签名的 `registry/`（raw 直连）。那个仓库刻意保持零代码（不含工具链、不含 CI）：改完模板后用本仓库的工具（`generate_registry` + `sign_registry`）重新生成并签名 `registry/`，与模板一起提交；校验用 `verify_templates` / `e2e_download` 指向模板库 checkout。
- **两级结构**：`manifests.json`（索引）+ `templates/<id>.json`（惰性拉取）。浏览只拉清单，导入才拉模板。
- **缓存与离线**：清单与模板带缓存 + ETag/If-None-Match 增量校验，断网时显示缓存并标注「离线」。
- **签名（可选）**：配置 `registry_pubkeys` 可强制对 `manifests.json` 做 Ed25519 验签。

## 开发

前置依赖：[Rust](https://rustup.rs/)（stable）、[Node.js](https://nodejs.org/)；macOS 需 Xcode CLT；Linux 需 Tauri 依赖（`libwebkit2gtk-4.1-dev` 等，见 [Tauri 前置](https://v2.tauri.app/start/prerequisites/#linux)）。

```bash
# Tauri（桌面应用 + 内嵌 Web 管理）
cd app-tauri && npm install && npm run tauri dev

# 仅无窗口 Web 管理服务
cargo run -p app-cli -- --port 0

# shared 库常用示例（模板库已拆到独立仓库：把 universal-shell-templates 克隆到
# 本仓库旁边，或用环境变量指向任意 checkout）
cargo run -p shared --example verify_templates -- ../universal-shell-templates/templates
cargo run -p shared --example generate_registry -- ../universal-shell-templates/registry ../universal-shell-templates/templates
cargo run -p shared --example sign_registry -- ../universal-shell-templates/registry/manifests.json
cargo run -p shared --example registry_demo -- ../universal-shell-templates/registry

# 打包（输出到 dist/，含 sha256.txt）
./scripts/build-release.sh 0.1.0
```

## 安全说明

- **信任模型**：远程模板会引导下载并执行第三方二进制。默认 `bind=127.0.0.1` + 高位端口；模板记录 `template_source`/`imported_at`。
- **下载校验**：优先模板 `check_sha256`，否则用 GitHub Releases `digest`（sha256）逐字节校验，失败即删除并报错。
- 不收集任何遥测，配置与日志只写本地。

## 许可

MIT — 详见 [LICENSE](LICENSE)。
