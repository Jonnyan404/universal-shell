# AGENTS.md

## 项目约定

- 本仓库即 universal-shell（配置驱动的万能应用壳）——核心逻辑在 `shared/`（纯 Rust，零 UI 依赖），`app-tauri/` 为主界面，`app-egui/` 为轻量备用界面。
- 模板库（下载规则）：`templates/*.json`，用 `cargo run -p shared --example generate_registry` 生成静态注册表。

## 工作习惯（必须遵守）

- **完成并验证通过的工作，必须立即 commit，这是一个好习惯，方便出问题时追溯。**
  - commit 之前先 `git add` 只纳入本批次相关文件，写清晰简洁的英文 commit message（祈使句，参考仓库历史风格，如 "Fix version check when config file is missing"）。
  - 用户授权可 push 时，commit 后一并 push（默认推 origin/main）。
- 改代码后运行对应检查：
  - Rust：`cargo build --all-targets`（须零警告）；表格/行为验证用 `cargo run -p shared --example verify_templates` 等。
  - Python：`python -m compileall` 或项目现有测试/检查方式。
- 大改动前先看 `ROADMAP.md` 的阶段规划与决策记录。

## 决策记录（摘要）

- UI 选型：**Tauri 为主界面**（模板浏览、前端迭代快），**egui 轻量备用**（托盘常驻场景）；核心逻辑 100% 在 `shared/`。
- 远程模板源：两级结构（`manifests.json` 索引 + `templates/<id>.json` 惰性拉取）；本地缓存 + ETag/If-None-Match + 断网回退缓存显示「离线」；导入时快照进 `shell.json` 并记录 `template_source`/`imported_at`。
- 安全默认值：模板 `bind=127.0.0.1`、高位端口。