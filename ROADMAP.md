# universal-shell 开发路线图

> 进度对照表。勾选已完成的项，改动后持续更新。

## 阶段 A — 核心能力扩展（本地，先做）

| # | 任务 | 验收标准 | 状态 |
|---|---|---|---|
| A1 | 资产匹配改为「候选列表」按序命中 | dufs 双平台候选正确解析 | ☑ |
| A2 | 解压支持 whole 整包模式（非 single） | syncthing 整包 + member 路径可启动 | ☑ |
| A3 | 资产模板标志：version 位置开关 | filebrowser（无版本号）能匹配 | ☑ (由候选模板占位符覆盖) |
| A4 | `verify_templates` 验证工具（cargo example） | 遍历模板调 GitHub API 报 PASS/FAIL | ☑ |
| A5 | 真实模板：dufs | 验证工具 PASS + e2e 可下载运行 | ☑ |
| A6 | 真实模板：syncthing（whole 模式） | 同上 | ☑ |
| A7 | 真实模板：frp（whole 模式） | 同上(verify PASS；e2e 与 A6 同路径已验证) | ☑ |

## 阶段 B — 远程模板源

| # | 任务 | 验收标准 | 状态 |
|---|---|---|---|
| B1 | `shell.json` 增加 `template_registries[]` 配置 | 多源配置可解析 | ☑ |
| B2 | 两级 fetch 客户端：清单 + 模板惰性拉取 | 浏览只拉清单，导入才拉模板 | ☑ |
| B3 | 本地缓存 + ETag/If-None-Match + 离线回退 | 断网显示缓存 + 「离线」标记 | ☑ |
| B4 | 导入流程：远程模板快照成实例，记录 `template_source`/`imported_at` | shell.json 里可见来源 | ☑ |
| B5 | 安全默认值字段：`bind=127.0.0.1`、高位端口 | 模板默认不暴露公网 | ☑ (模板均已 bind 127.0.0.1) |
| B6 | 开源注册表托管方案落地（GH repo + Pages/raw 静态 JSON） | URL 可直接被客户端拉取 | ☑ |
| B7 | 模板选择器 UI：浏览/搜索/刷新/来源显示/「拉取失败回退缓存」 | egui 版可浏览导入 | ☑ |
| B8 | 同功能 Tauri 版前端 | 同上 | ☑ |
| B9 | 多 Tab 多实例管理（egui + Tauri） | 同模板多实例独立启停 | ☑ |

## 阶段 C — 加固与 CI

| # | 任务 | 验收标准 | 状态 |
|---|---|---|---|
| C1 | 模板库扩充至 10~15 个（分类：文件共享/代理/存储/…) | verify 工具全 PASS | ☑ (12 个，verify+e2e 全过) |
| C2 | sha256 digest 校验（可选 `check_sha256`） | 篡改资产下载被拒 | ☑ (GitHub API digest + 模板 check_sha256 双通道，verify 全 PASS) |
| C3 | GitHub Actions 定时跑 verify_templates | 资产命名变化自动失败告警 | ☑ (verify-templates.yml 每日+PR) |
| C4 | 实例 vs 模板版本 diff + 「应用模板更新」 | 更新保留用户已填字段 | ☑ (template_diff/apply_template_update + egui 入口+测试) |
| C5 | 多 registry 合并与 id 冲突去重 | 双源无冲突导入 | ☑ (load_merged_manifests + egui/Tauri 双端多源列表+冲突标记) |
| C6 | 远程库签名验签(可选) | 签名校验通过才启用 | ☑ (Ed25519 签名 manifests.sig + registry_pubkeys 强校验, registry_demo 全验) |

## 阶段 D — 发布与体验

| # | 任务 | 验收标准 | 状态 |
|---|---|---|---|
| D1 | 托盘/最小化到托盘（tray-icon） | 最小化隐藏，托盘可唤出/退出 | ☑ (Tauri: 内置 tray-icon feature + 关闭即隐藏 + 托盘菜单显隐/退出; egui: tray-icon crate 主线程建盘 + 后台线程转发菜单/点击) |
| D2 | 打包：egui 单二进制 + tauri bundle | 两版可分发 | ☑ (scripts/build-release.sh: egui 单二进制 + cargo tauri build 产出 .app/.dmg 与 sha256.txt，产物架构/DMG 校验通过) |
| D3 | README 更新（模板源、配置、安全说明） | — | ☑ (README 补全：远程模板库、packaging、配置含 template_registries、校验/签名/安全说明) |

## 执行顺序建议

A（1→7）→ B1-B3 → B4/B7（B8）→ B9 → C1/C3 → C4/C2/C5 → C6 → D1-D3

## 关键决策记录

- **前端**：egui 与 Tauri 双实现，核心逻辑全部在 `shared/` 复用
- **主前端**：Tauri 为主界面（模板浏览/搜索体验 + 前端迭代快），egui 为轻量备用（托盘常驻场景）
- **模板源**：两级结构（清单 + 模板惰性拉取），本地缓存可离线回退
- **信任模型**：远程模板会引导下载并执行第三方二进制；默认安全字段值 + 来源记录 + 可选 sha256/签名