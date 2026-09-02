# universal-shell (Rust)

用 Rust 重写的「配置驱动二进制程序管理壳」——同时提供 **egui** 与 **Tauri** 两个 GUI 前端，共享同一个核心库 (`shared`)。

- 从 GitHub 自动下载/更新受管程序二进制，解压落盘（与壳自身不同名，天然防覆盖）
- 下载后校验：优先模板 `check_sha256` pin，否则用 GitHub Releases API 提供的 `digest`（sha256）逐字节校验
- 启动/停止子进程、参数模板替换
- **配置驱动 UI**：`shell.json` 里的 `fields` 决定界面渲染什么控件：
  - `string` — 文本框
  - `file` — 文件路径 + 原生浏览按钮 (rfd / Tauri dialog)
  - `directory` — 目录路径 + 原生浏览按钮
  - `boolean` — 复选框
  - `autostart` — 开机启动复选框（写入系统 LoginItem / .desktop / registry）
- 多程序多 Tab，每个程序独立数据目录
- 托盘常驻：关闭窗口即隐藏到托盘，托盘左键/菜单可唤出，菜单可退出（Tauri 与 egui 双端）
- 远程模板库：浏览只拉清单、导入才拉模板，本地缓存可离线回退，多源合并时 id 冲突按源顺序去重并标记

## 结构

```
├── Cargo.toml            # workspace (shared / app-egui / app-tauri)
├── shared/               # 核心库：配置模型、GitHub、下载、解压、进程、开机启动、模板库、签名
│   └── examples/         # verify_templates / generate_registry / sign_registry / registry_demo / e2e
├── app-egui/             # eframe/egui 前端 (≈ 原生小体积)
├── app-tauri/            # Tauri v2 前端 (WebView 渲染)
├── templates/            # 手写模板源 (*.json)
├── registry/             # 生成的发布注册表（manifests.json + manifests.sig + templates/）
├── scripts/              # 发布打包脚本
└── demo/shell.json       # 示例配置（cloud-clipboard-go + 演示程序）
```

## 开发环境

### 前置依赖

- [Rust](https://rustup.rs/)（stable）
- [Node.js](https://nodejs.org/)（Tauri 前端需要）
- macOS 额外：Xcode Command Line Tools（`xcode-select --install`）
- Linux 额外：见 [Tauri Linux Prerequisites](https://v2.tauri.app/start/prerequisites/#linux)，核心是 `libwebkit2gtk-4.1-dev`、`libappindicator3-dev` 等

### 以开发模式运行

#### egui 版（推荐先用这个，启动最快）

```bash
cargo run -p app-egui -- demo/shell.json
```

使用 `demo/shell.json` 作为示例配置，加载后可浏览模板、安装程序。

#### Tauri 版

```bash
cd app-tauri && npm install && npm run tauri dev
```

首次运行会同时编译 Rust 后端和启动前端 dev server，之后增量编译较快。

### 其他常用开发命令

```bash
# 仅编译全部 crate（检查错误/警告）
cargo build --all-targets

# 运行 shared 库的示例
cargo run -p shared --example verify_templates
cargo run -p shared --example generate_registry
cargo run -p shared --example sign_registry

# 生成 UI 文件（Tauri 前端改动后无需手动操作，Vite 自动处理）
# 如需手动生成：cd app-tauri/src-tauri && cargo build
```

## 打包发布

```bash
./scripts/build-release.sh 0.1.0
```

产物输出到 `dist/`：

- `universal-shell-egui-<version>-<os>-<arch>` — egui 单二进制
- `bundle/...`（含 .app / .dmg 等系统原生安装包）
- `sha256.txt` — 各产物 SHA256，供发布时核验

## 远程模板库

- **两级结构**：`manifests.json` 是索引（只列 id/名称/描述/分类/版本号），具体模板在 `templates/<id>.json`；客户端浏览时只拉清单，导入时才惰性拉取模板。
- **缓存与离线**：清单与模板都做本地缓存 + ETag/If-None-Match 增量校验；断网时显示缓存内容并标注「离线」。
- **多源合并**：`shell.json` 的 `template_registries[]` 可配多个源；同 id 冲突时按源顺序第一个优先，界面标出冲突来源。
- **托管**：本仓库 `registry/` 即静态注册表，可用 GitHub raw 直接拉取；示例见 `demo/shell.json`。修改 `templates/` 后需重跑 `cargo run -p shared --example generate_registry` 重新生成 `registry/`。
- **签名校验**：可选。配置了 `registry_pubkeys`（base url → Ed25519 公钥 hex）时，客户端拉取 `<base>manifests.sig` 并对清单字节验签，失败则拒收该源。生成签名用 `cargo run -p shared --example sign_registry`。

## 配置格式 (`shell.json`)

字段类型由 `fields[].kind` 决定。`args` 模板用 `{key}` 引用字段值。

```jsonc
{
  "template_registries": ["https://raw.githubusercontent.com/Jonnyan404/universal-shell/main/registry/"],
  "programs": [
    {
      "id": "cloud-clipboard-go",
      "name": "cloud-clipboard-go",
      "description": "云剪贴板核心程序",
      "repo": "Jonnyan404/cloud-clipboard-go",
      "binary": "cloud-clipboard-go",          // 落盘名（必须与壳不同名）
      "assets": {                               // 按 OS 匹配资产
        "darwin":  { "filename": "{name}_Darwin_{arch}.tar.gz",  "format": "tar.gz", "member": "cloud-clipboard-go" },
        "linux":   { "filename": "{name}_Linux_{arch}.tar.gz",   "format": "tar.gz", "member": "cloud-clipboard-go" },
        "windows": { "filename": "{name}_Windows_{arch}.zip",    "format": "zip",     "member": "cloud-clipboard-go.exe" }
      },
      "arch_map": { "x86_64": "x86_64", "aarch64": "arm64" },
      "fields": [
        { "key": "host",      "kind": "string",    "label": "监听地址", "default": "0.0.0.0" },
        { "key": "port",      "kind": "string",    "label": "端口",     "default": "9000" },
        { "key": "config",    "kind": "file",      "label": "配置文件", "filter": "*.json,*.yaml,*.toml" },
        { "key": "data_dir",  "kind": "directory", "label": "数据目录" },
        { "key": "verbose",   "kind": "boolean",   "label": "详细日志", "default": false },
        { "key": "autostart", "kind": "autostart", "label": "开机启动", "default": false }
      ],
      "args": ["-host", "{host}", "-port", "{port}", "-config", "{config}", "-dir", "{data_dir}"]
    }
  ]
}
```

占位符：资产 `filename` 支持 `{name}` `{version}` `{arch}` `{ext}`；`args` 支持任意 `{fieldKey}`。

## 数据目录

默认 `~/Library/Application Support/universal-shell/`（Windows: `%APPDATA%/universal-shell/`）。
每个受管程序落地为：
- `<data>/<binary>` — 可执行文件（Unix 自动 +x）
- `<data>/<id>.version` — 本地版本号
- `<data>/<id>.values.json` — 上次表单值
- `<data>/logs/<id>.{out,err}.log` — 运行日志

## 核心库 API

```rust
use shared::ShellManager;

let mut mgr = ShellManager::new(data_dir)?;
mgr.load_config(&config_path)?;

// 安装/更新（供线程使用，不持有 manager 锁）
let version = ShellManager::install_standalone(&data_dir, &program)?;

// 启动/停止（自动用 args 模板替换字段值）
mgr.start(&program, &field_values)?;
mgr.stop(&program.id)?;

// 开机启动
mgr.apply_key_autostart(&program, &field_values)?;
```

## 对比小结

| | egui (`app-egui`) | Tauri (`app-tauri`) |
|---|---|---|
| 渲染 | 原生绘制，无 WebView | 系统 WebView |
| 体积 | ~20 MB 级，单二进制 | 依赖 webview（macOS 内建） |
| 动态表单 | 纯代码遍历 fields 渲染 | JS 渲染 + 后端命令 |
| 文件选择 | rfd（原生对话框） | tauri dialog 插件 |
| 托盘 | tray-icon crate | tauri 内置 tray-icon |
| 适合 | 轻量、托盘常驻、纯工具风 | 界面精致、需要前端组件生态 |

## 安全说明

- **信任模型**：远程模板会引导下载并执行第三方二进制。默认要求：监听类程序 `bind=127.0.0.1` + 高位端口；模板记录 `template_source`/`imported_at` 便于回溯。
- **下载校验**：模板可选 `check_sha256` 硬编码摘要（优先级最高）；否则用 GitHub Releases API 返回的 `digest` 校验。校验失败即删除文件并报错。
- **远程签名**：配置 `registry_pubkeys` 后可对 `manifests.json` 强制 Ed25519 验签，公钥不在配置内的源直接拒收。
- **不收集任何遥测**，配置与日志只写本地数据目录。

## 待补充

- [ ] 下载进度回调（install 目前以阻塞方式完成）
- [ ] mac 签名 / notarize 工作流