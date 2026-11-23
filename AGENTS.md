# AGENTS.md

## 项目约定

- 本仓库即 universal-shell（配置驱动的万能应用壳）——核心逻辑在 `shared/`（纯 Rust，零 UI 依赖），`app-tauri/` 为主界面（单一前端 SPA + 内嵌 web 服务），`app-cli/` 为无窗口 Web 管理服务（`us-web`）。
  - `app-egui/` **已归档**：源码留在仓库备查，但已在 `Cargo.toml` 里 `exclude`，且不参与打包/CI。**不要再给它加功能**；要复活先看 `ROADMAP.md` 关键决策记录里的复活条件。
- `us-service/`（lib `us_service` + 二进制 `us-svc`）是**无 UI 依赖**的服务调度 crate（不链接 axum/tokio，看板只在主 SPA 的工作台里）：把 `services.toml` 生成 launchd / systemd / 计划任务配置，并由 `us-svc run` 包装器负责日志与退出码。它不关心程序从模板下载还是用户自写；改它不影响 `shared/`，反之亦然。
- **模板库已拆到独立仓库** [universal-shell-templates](https://github.com/Jonnyan404/universal-shell-templates)：
  - 那里是模板单一事实来源（`templates/*.json`）与生成产物（`registry/`：`manifests.json` + `manifests.sig` + `templates/<id>.json`）。
  - 本仓库**不再存放模板**，只保留 schema（`shared/src/config.rs`）与工具链。加/改模板请去那个仓库提 PR。
  - 工具链用法（模板库目录用参数或环境变量传入，本地一般把模板库 clone 到本仓库旁边）：
    - 生成：`cargo run -p shared --example generate_registry -- <out_dir> <templates_dir>`（env：`US_REGISTRY_OUT_DIR` / `US_TEMPLATES_DIR`）
    - 校验：`cargo run -p shared --example verify_templates -- <templates_dir...>`（env：`US_TEMPLATES_DIRS`）
    - 端到端下载：`US_TEMPLATES_DIR=... cargo run -p shared --example e2e_download -- <id...>`
    - 签名与客户端行为自测：`sign_registry` / `registry_demo`（env：`US_REGISTRY_DIR`）
  - 模板库仓库**零代码、零依赖、无 CI**（这样它不与本仓库产生任何构建期耦合，也就不需要跨仓库凭据）：生成/签名/校验都在本仓库侧跑，产物 `registry/` 与模板一起提交；模板库自身不带生成逻辑。
  - 例：`cargo run -p shared --example generate_registry -- ../universal-shell-templates/registry ../universal-shell-templates/templates`
- **壳不内置任何程序**：`shared/builtin/` 已删除（2026-11-23），全新数据目录的程序列表为空，一切程序都来自模板库导入或手动新建。模板库是唯一的内容来源。

## 工作习惯（必须遵守）

- **完成并验证通过的工作，必须立即 commit，这是一个好习惯，方便出问题时追溯。**
  - commit 之前先 `git add` 只纳入本批次相关文件，写清晰简洁的英文 commit message（祈使句，参考仓库历史风格，如 "Fix version check when config file is missing"）。
  - **常设授权**：完成并验证通过的工作，commit 后直接 push（默认 `origin/main`），不必再逐次征求同意。推送前必须先把对应检查跑绿（见下），不要推未验证的改动。
- 改代码后运行对应检查：
  - Rust：**不要 `cargo build`**，最多用 `cargo run -p <crate>` 以开发模式运行调试；表格/行为验证用 `cargo run -p shared --example verify_templates` 等。
  - Python：`python -m compileall` 或项目现有测试/检查方式。
- 大改动前先看 `ROADMAP.md` 的阶段规划与决策记录。

## 决策记录（摘要）

- UI 选型：**Tauri 为主界面**（模板浏览、前端迭代快），**egui 轻量备用**（托盘常驻场景）；核心逻辑 100% 在 `shared/`。
- 远程模板源：两级结构（`manifests.json` 索引 + `templates/<id>.json` 惰性拉取）；本地缓存 + ETag/If-None-Match + 断网回退缓存显示「离线」；导入时快照进 `shell.json` 并记录 `template_source`/`imported_at`。
- 安全默认值：模板 `bind=127.0.0.1`、高位端口。