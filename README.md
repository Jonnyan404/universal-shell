# universal-shell

A configuration-driven manager for third-party CLI programs **and local services**: download, configure, run — either app-managed or handed to the OS scheduler. A **Tauri** desktop GUI and a headless web manager share one core library (`shared`).

**English** · [中文](#中文文档)

- Downloads/updates program binaries from GitHub into its own data dir (never overwrites itself), verifies SHA256, spawns/stops processes with templated args.
- **One workbench**: programs and services in a single list. Each program is either **app-managed** (lives and dies with the app) or handed to the **OS scheduler** (can start at login via the autostart switch, restarts on crash, ordered by declared dependencies).
- **Config-driven UI**: the `fields` in `shell.json` decide which widgets render (string / file / directory / boolean).
- Multiple programs, each with its own data dir.
- Tray resident: closing the window hides to tray; tray can reopen or quit.
- **Same UI everywhere**: the desktop window and any browser load the same single-page app — the tray is the only desktop-only piece.

---

## Quick start

### 0. Pick an interface

| | Tauri (`app-tauri`) | CLI (`us-web`, `app-cli`) |
|---|---|---|
| What it is | Full-featured desktop app (system WebView) | Headless web manager, no window |
| Best for | Polished UI / rich frontend | NAS, Raspberry Pi, headless server |
| Rendering | System WebView | Browser only |
| File picker | tauri dialog plugin | manual / upload via web |

Both serve the **same SPA**: the Tauri window is a native shell (window + tray) around it, so anything you can do in the browser you can do in the app, and vice versa. They share one config, data directory and template library — switch between them anytime.

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

> Not every program needs a template. **＋ New → New program** opens a form for a hand-written program (id, name, command, args, env, fields); **＋ New → New service** writes a single command into `services.toml`. The template library stays a separate sidebar entry. You can also hand-edit `shell.json` (see [Config](#config)).

### 3. Use

Set the fields if needed, then press **Start**. By default the program is app-managed: it runs in the background and stops when you quit the app. The **⏻ Enable/Disable autostart** button in the action bar is the single "start at login" switch: for app-managed programs the shell launches them at login, and for the OS scheduler it decides whether the derived startup item is registered. To make the program survive the app, see the next section.

### 4. Two ways to keep a program running

| | App-managed | OS scheduler |
|---|---|---|
| Who starts it | this app | launchd / systemd / Task Scheduler |
| When the app quits | the program stops | it keeps running |
| At login | if the app autostarts and the autostart switch is on | if the autostart switch is on — registered as a startup item |
| After a crash | no restart | restarts per your setting |
| Good for | tools you want to watch | services that must always be up |

Switch under **Schedule** in the detail pane. The start/stop/restart buttons still work after
handover — they go through the OS scheduler (`native_dispatch`) instead of spawning a second
copy from the app — and a **Back to app-managed** button is added. Handing a program over stops
the copy the app was running first, so the scheduler's instance never ends up beside it.

**Stop** on a scheduler-managed program really stops it: on macOS the startup item is unloaded
(`launchctl bootout`) rather than signalled, because `KeepAlive` would bring a signalled process
back within seconds. The item stays on disk, so the next login still starts it — same meaning as
`systemctl stop` leaving a unit enabled. **Start** loads it again; if the autostart switch is off
(no startup item on disk) starting it must register one, so the switch is turned back on and the
app says so instead of changing it silently.

**One autostart switch, two mechanisms.** The switch is shell-managed state
(`program-autostart.json`) and works in both modes. Handing a program to the OS scheduler for
the first time turns it on automatically — that is what "starts at login" means there. Turning
it off afterwards unregisters the launchd/systemd/Task Scheduler item and marks the derived
service `enabled = false`; turning it back on reloads it. Saving schedule parameters or
switching the supervisor never resets a switch you turned off yourself.

**Dependencies**: `depends_on = ["db"]` waits until the dependency is ready — its port is
reachable, a readiness file exists, or (when neither is declared) the process is running.
A timeout logs a warning and continues, so one broken dependency cannot stall everything else.

Deleting a program unregisters its startup item as well, so nothing is left running behind your back.

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

To hand a program's lifetime to the OS scheduler instead of the app:

```jsonc
"supervisor": "native",   // default "managed"
"native": { "delay_secs": 10, "restart": "on_failure", "scope": "user", "depends_on": ["db"] }
```

`native` entries are derived into `services.toml` on every sync; that file is the projection, `shell.json` stays the source of truth.

## Data directory

Default: `~/Library/Application Support/universal-shell/` (macOS), `~/.local/share/universal-shell/` (Linux, honours `$XDG_DATA_HOME`), `%APPDATA%/universal-shell/` (Windows). Each managed program:

- `<data>/<binary>` — the executable
- `<data>/<id>.version` — local version
- `<data>/<id>.values.json` — last form values
- `<data>/logs/<id>.{out,err}.log` — logs captured by the app

Shared files:

- `<data>/services.toml` — service declarations: hand-written ones plus those derived from programs (`origin = "program:<id>"`)
- `<data>/logs/<id>.log` — output of scheduler-run services (stdout+stderr, exit codes), written by the wrapper

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

# Inspect services without any UI (built from source; not shipped)
cargo run -p us-service --bin us-svc -- list   services.toml
cargo run -p us-service --bin us-svc -- order  services.toml   # dependency order
cargo run -p us-service --bin us-svc -- status services.toml

# Package (outputs to dist/, includes sha256.txt)
./scripts/build-release.sh 0.1.0
```

Release builds do **not** include a separate `us-svc` binary: `us-service` is a library the app
already links, and the wrapper role is played by whichever executable the scheduler is pointed at
(`us-web run --config … --id …`, or the desktop binary's `run`).

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
- **Startup items**: handing a program to the scheduler writes a launchd plist / systemd unit / Task Scheduler XML. Deleting the program (or switching back to app-managed) unregisters it.
- No telemetry; config & logs stay local.

## License

MIT — see [LICENSE](LICENSE).

---

<a name="中文文档"></a>

# universal-shell（中文）

配置驱动的管理壳，面向第三方命令行程序**与本地服务**：下载、配置、运行——可以交给应用托管，也可以交给系统调度。**Tauri** 桌面界面与无窗口 Web 管理服务共享同一个核心库 (`shared`)。

[English](#universal-shell) · 中文文档

- 从 GitHub 自动下载/更新受管程序二进制到独立数据目录（天然防覆盖），SHA256 校验、参数模板替换、启动/停止子进程。
- **一个工作台**：程序与服务同列。每个程序的运行方式二选一——**壳托管**（随应用启停），或交给**系统调度**（可开机自启、崩溃重启、按声明的依赖顺序启动）。
- **配置驱动界面**：`shell.json` 里的 `fields` 决定渲染什么控件（string / file / directory / boolean）。
- 多程序并存，每个程序独立数据目录。
- 托盘常驻：关窗即隐藏到托盘，托盘可唤出/退出。
- **界面只有一份**：桌面窗口与任意浏览器加载的是同一个单页应用，只有托盘是桌面端独有。

---

## 快速上手

### 0. 先选界面

| | Tauri（`app-tauri`） | CLI（`us-web`，`app-cli`） |
|---|---|---|
| 是什么 | 完整桌面应用（系统 WebView） | 无窗口 Web 管理服务 |
| 适合 | 界面精致 / 前端生态 | 树莓派 / NAS / 无桌面服务器 |
| 渲染 | 系统 WebView | 仅浏览器 |
| 文件选择 | tauri dialog 插件 | 手填 / 网页上传 |

两者跑的是**同一份单页应用**：Tauri 窗口只是它外面的一层原生壳（窗口 + 托盘），浏览器里能做的事桌面端都能做，反之亦然。配置、数据目录与模板库共用，可随时切换。

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

> 本地程序不必来自模板。点「**＋ 新建**」：选「**新建程序**」手动填 id、名称、命令、参数、环境变量与字段；选「**新建服务**」写一条命令进 `services.toml`。模板库仍是侧栏的独立入口。也可以直接在 `shell.json` 里手写（见下方 [配置格式](#配置格式-shelljson)）。

### 3. 使用

需要时填好字段，点 **启动**。默认是壳托管：程序在后台跑，应用一退它就停。操作区的「**⏻ 启用开机自启 / 取消开机自启**」是「是否开机启动」的唯一开关：壳托管时由壳在启动时代为拉起，系统调度时决定是否为它注册系统启动项。想让程序活过应用，见下一节。

### 4. 两种运行方式

| | 壳托管 | 系统调度 |
|---|---|---|
| 谁拉起 | 本应用 | launchd / systemd / 计划任务 |
| 应用退出 | 程序一起停 | 继续运行 |
| 登录时 | 应用自启且该程序开了自启开关 | 开了自启开关才会，已注册为系统启动项 |
| 崩溃后 | 不会重启 | 按你的设置重启 |
| 适合 | 你想盯着看的工具 | 必须一直开着的服务 |

在详情页的「**调度**」里切换。交出去之后**启停/重启仍然可用**——它们走 `native_dispatch` 直接操作系统调度器（launchctl / systemctl / 计划任务），不经过应用自己的进程管理；交接时会先把壳正在跑的那份停掉，不会和调度器双开。操作区只多一个「**⏻ 改为壳托管**」按钮。

**停止**是真的停：macOS 上走 `launchctl bootout` 卸载作业，而不是给进程发信号——`KeepAlive` 会在几秒后把被杀的进程重新拉起来（实测约 8 秒）。作业文件仍留在磁盘上，所以下次登录照样会起来，这与 `systemctl stop`（保持 enabled）是同一个语义。**启动**会重新装载它；若此时「开机自启」开关是关着的（磁盘上没有作业文件），装载本身就意味着注册开机启动，因此会顺带把开关打开，并明确告诉你，而不是悄悄改。

**一个开关、两套机制。** 开关是壳统一管理的状态（`program-autostart.json`），两种托管方式下都有效。首次把程序交给系统调度时会自动打开它（「开机自启」本来就是这个模式的语义）；之后关掉会注销 launchd / systemd / 计划任务里的启动项，并把派生服务标成 `enabled = false`，再打开则重新装载。保存调度参数或切换托管方式都不会把你手动关掉的开关重置回来。

**依赖顺序**：`depends_on = ["db"]` 会等到依赖「就绪」——端口可连、就绪文件出现，或（两者都没声明时）进程在跑。
等满超时只记警告并继续，避免一个坏依赖把整条启动链卡死。

删除程序时，它的系统启动项会一并注销，不会在后台留下一个看不见的服务。

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

把程序的生命周期交给系统调度：

```jsonc
"supervisor": "native",   // 默认 "managed"
"native": { "delay_secs": 10, "restart": "on_failure", "scope": "user", "depends_on": ["db"] }
```

`native` 条目会在每次同步时派生进 `services.toml`；那是投影，`shell.json` 始终是唯一事实来源。

## 数据目录

默认 `~/Library/Application Support/universal-shell/`（macOS）、`~/.local/share/universal-shell/`（Linux，遵循 `$XDG_DATA_HOME`）、`%APPDATA%/universal-shell/`（Windows）。每个受管程序：

- `<data>/<binary>` — 可执行文件
- `<data>/<id>.version` — 本地版本号
- `<data>/<id>.values.json` — 上次表单值
- `<data>/logs/<id>.{out,err}.log` — 应用捕获的日志

共用文件：

- `<data>/services.toml` — 服务声明：手写服务，以及从程序派生的条目（`origin = "program:<id>"`）
- `<data>/logs/<id>.log` — 由系统调度运行的服务输出（stdout+stderr、退出码），由包装器写入

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

# 不启界面地查看服务（本机编译，发布包不带它）
cargo run -p us-service --bin us-svc -- list   services.toml
cargo run -p us-service --bin us-svc -- order  services.toml   # 依赖顺序
cargo run -p us-service --bin us-svc -- status services.toml

# 打包（输出到 dist/，含 sha256.txt）
./scripts/build-release.sh 0.1.0
```

发布产物里**没有**独立的 `us-svc`：`us-service` 是被应用链接的库，包装器由调度器指向的那个可执行文件承担
（`us-web run --config … --id …`，或桌面端的 `run`）。

## 安全说明

- **信任模型**：远程模板会引导下载并执行第三方二进制。默认 `bind=127.0.0.1` + 高位端口；模板记录 `template_source`/`imported_at`。
- **下载校验**：优先模板 `check_sha256`，否则用 GitHub Releases `digest`（sha256）逐字节校验，失败即删除并报错。
- **系统启动项**：把程序交给调度器会写入 launchd plist / systemd unit / 计划任务 XML；删除程序（或切回壳托管）会一并注销。
- 不收集任何遥测，配置与日志只写本地。

## 许可

MIT — 详见 [LICENSE](LICENSE)。
