---
name: github-template-json
description: Use when the user hands you a GitHub repo/release URL (or owner/repo) and wants a universal-shell program template, i.e. a templates/<id>.json file for this project's template library. Generates a schema-valid Program JSON with darwin/linux/windows asset rules by querying the GitHub API for real release assets, then writes it to templates/ and validates it with cargo run -p shared --example verify_templates.
license: MIT
---

# universal-shell 模板生成

把任意 GitHub 程序仓库转成 universal-shell 的模板文件 `templates/<id>.json`。
模板是「配置驱动的万能应用壳」里描述如何下载、安装、启动一个程序的结构化 JSON，
会被导入成受管程序（menus→库里可安装）。schema 见下文，务必逐字段核对。

## 触发场景

- 用户给一个 GitHub 链接（仓库主页 / release 页 / 单 asset 链接），或 `owner/repo`，
  或只报一个程序名，要求「生成模板 / 加进模板库 / 做成模板」。
- 产出物 = 一个完整、schema 合法、能通过 `verify_templates` 校验的 JSON 文件。

## 模板 schema（Program，权威定义见本仓库 `shared/src/config.rs`）

根对象所有字段：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | string | 是 | 唯一 id，同时用作配置文件名 / 下载目录名，_全小写_（如 `croc`） |
| `name` | string | 是 | 显示名 |
| `description` | string | 否 | 中文一句话简介 + 默认绑定安全提示（若监听网络） |
| `category` | string | 是 | 现有分类：`file-sharing` `proxy` `sync` `vcs` `clipboard` `utility`（必要时可新增合理值） |
| `repo` | string | 是 | `owner/repo` |
| `binary` | string | 是 | 落盘可执行文件名（刻意与壳不同名） |
| `assets` | object | 是 | 键 = `darwin` `linux` `windows`，值 = AssetRule |
| `arch_map` | object | 否 | 本机 arch(x86_64/aarch64) → 上游资产里的 arch token |
| `os_map` | object | 否 | 本机 os → 上游 os token（如 `{"macos":"apple-darwin"}`） |
| `fields` | array | 是 | UI 字段定义（见下） |
| `args` | array | 是 | 启动参数模板，`{key}` 会被字段值替换 |
| `working_dir` | string | 否 | 默认 `.` |
| `check_sha256` | string | 否 | 可选，钉住期望 sha256；缺省用 GitHub 返回的 asset digest |
| `hidden` | bool | 否 | 默认省略 |

### AssetRule（assets 每个平台的值）

```jsonc
{
  "candidates": ["croc_v{version}_macOS-ARM64.tar.gz", "croc_v{version}_macOS-64bit.tar.gz"],
  "format": "tar.gz",        // tar.gz | zip | raw
  "mode": "single",          // single | whole | raw
  "member": "croc"           // single: 包内成员名；whole: 包内相对路径；raw/留空可省略
}
```

- `candidates`：按序尝试、第一个真实存在者胜出的**文件名模板**。支持占位符：
  `{name}`(id) `{version}`(tag 去前导 v) `{arch}`(arch_map 映射后的 token) `{os}`(os_map 映射后的 token) `{ext}`(tar.gz 或 format 本身)。
  注意 **`{name}` 渲染成 id**（见 config.rs render_template），所以模板里文件名前缀一般直接写库名或 `{name}` 均可，但写实际名字更稳。
- `mode`：
  - `single` 抽单成员（默认）
  - `whole` 整包解到 id 目录（用于带版本目录、需保留目录结构的包，如 syncthing）
  - `raw` 裸二进制 / 自解压 exe（gitea、单文件）
- 若上游同时提供 `tar.gz` 与原始 binary，优先压缩包。

### fields / args

`kind` 决定控件：`string` `file` `directory` `boolean` `autostart`。
字段结构（`key` + kind 相关字段）：

- `string`：`{ key, kind:"string", label, default:"", placeholder:"" }`
- `file`：`{ key, kind:"file", label, default:"", filter:"*.json" }`（rfd 过滤）
- `directory`：`{ key, kind:"directory", label, default:"" }`
- `boolean`：`{ key, kind:"boolean", label, default:false }`
- `autostart`：`{ key, kind:"autostart", label, default:false }`（特殊：写系统 LoginItem/自启）
- 任一字段可加 `"required": true` 标记为**必填**：值为空时启动前校验报错，**管理页表单会把必填字段标签标红显示**（末尾带 `*`）。
  - **何时标 required（重点考虑）**：凡字段**缺省为空、且为空会导致程序启动失败或无意义运行**，都该标必填——
    配置文件（sing-box `config`、mihomo `config`、frpc `config`）、必须指定的目录/路径（filebrowser `root`、miniserve `dir`、gitea `worktree`）、
    必须的源/目标（rclone `src`、age `recipient`/`input`）等。
  - **不要标**：给了合理 `default` 的 `bind`/`addr`/`port`；纯透传、空值只是显示帮助的 `args_extra`（如 croc/lazygit/syncthing）。
  - 参照已有模板里的 required 用法：`dufs`(dir)、`cloud-clipboard-go`(host/port)、mihomo/sing-box/frpc(config)、age(recipient/input)。

`args` 是字符串数组，`{key}` 会被对应字段值替换（参照现有模板）。

常用模式：
- 网络服务类（file-sharing/utils、监听端口）：`bind`/`addr`+`port`+`autostart` 字段，
  args 如 `["-b","{bind}","-p","{port}",...]`。安全默认 `bind/addr = "127.0.0.1"`、高位端口。
- 有配置文件的上游：加 `file` 字段（如 sing-box 的 `config.json`）。
- 单次运行 TUI / 一条命令工具：加个 `args_extra` 透传参数即可。

## 工作流程

1. **解析仓库**：取 URL 中的 `owner/repo`。releases 页/asset 链接也归约到 repo。
2. **查真实资产（务必做，别猜）**：
   ```bash
   curl -sL "https://api.github.com/repos/{owner}/{repo}/releases/latest"
   ```
   若 latest 不适用，可列全部 release：`.../releases` 取最新 tag。用 `jq` 提取
   `tag_name` 与 `assets[].{name, browser_download_url, digest}`。
   无 `GITHUB_TOKEN` 时匿名 60 次/小时限额，够用。
3. **推导文件模板**：把每个真实文件名（去版本号/架构片段）替换回占位符，得到 `candidates`。
   - 版本号镶嵌处 → `{version}`（如 `croc_v9.6.17_macOS-ARM64.tar.gz` → `croc_v{version}_macOS-ARM64.tar.gz`）
   - 架构片段 → 据文件名里出现的 token 决定 `arch_map`：`x86_64`→上游 x86 token、`aarch64`→上游 arm token。
     多条候选按「当前 arch 中含该 token」优先排序（即把匹配本架构的放前），多架构时务必同时列出全部架构候选。
   - 读取上游 README/release 说明确认：解压后可执行文件名（member）、是压缩包还是裸 binary、
     是否需要整包解压（whole）等。
4. **填 metadata**：`id`（全小写、逻辑名）、`description`（中文一句 + 若监听网络注明默认绑定 127.0.0.1 的安全提示）、
   `category`（取最贴切现有值）、`binary`（通常 = member 或无压缩时的可执行名）。
5. **写 `fields`/`args`**：按上面启发式；不确定上游启动参数时最少给 `args_extra`，label 用中文。
6. **落盘**：写 `templates/<id>.json`（JSON 缩进，键序参照现有模板：id,name,description,repo,binary,category,assets,arch_map,os_map,fields,args）。
7. **校验**（本仓库根目录）：
   ```bash
   cargo run -p shared --example verify_templates
   ```
   该命令对当前 OS/arch 调 GitHub API 逐个核对候选资产能否命中真实 release，
   任一 FAIL 即视为还需修正（多半是 candidates/arch_map 写错）。可先看该模板单条输出。
8. **提交**：确认通过后，`git add templates/<id>.json` 并提交（英文祈使句 message，如
   `Add template for <name>`）。仅当用户授权 push 才 push。

## 完整示例（本仓库内已有，可直接参照）

- `templates/croc.json`、`templates/dufs.json`（tar.gz + single，darwin/linux/windows）
- `templates/gitea.json`（raw + raw 示例，双 OS arch_map）
- `templates/sing-box.json`（tar.gz/zip，`file` 字段加载 config 示例）
- `templates/syncthing.json`（whole 模式示例）

> 若某次校验失败且原因非人为疏漏（上游资产命名近期变更），保留真实资产名为准，
> 更新 candidates 使其与上游一致，并如实告知用户。

## 注意

- **一定以 GitHub API 真实返回的资产名为准**，不要凭印象写文件名。
- 多架构候选都列全；每个 OS 至少给 `x86_64` 与 `aarch64` 对应候选（能确定的就都给）。
- 只要没把握，就回读本仓库 `shared/src/config.rs` 与 `templates/*.json` 对齐 schema，再生成。
- 模板里的 `description`/字段 `label`/`placeholder` 用中文，与现有模板一致。
