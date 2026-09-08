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
| D2 | 打包：egui + tauri 各平台安装包 | 两版可分发，Release 发布时多平台矩阵打包 | ☑ (scripts/build-release.sh：macOS egui=通用 .app+.dmg，Windows=egui .exe+.msi / tauri=.app/.msi；GitHub Actions release 事件触发，矩阵含 linux/macOS/windows 的 x64+arm64，egui 安装包含 ARM64 Windows runner) |
| D3 | README 更新（模板源、配置、安全说明） | — | ☑ (README 补全：远程模板库、packaging、配置含 template_registries、校验/签名/安全说明) |

## 阶段 E — 本地实例管理（Tauri）

| # | 任务 | 验收标准 | 状态 |
|---|---|---|---|
| E1 | Program.hidden：侧栏/主管理隐藏，批量管理仍可见 | 隐藏程序从侧栏消失，批量管理可找回并切换 | ☑ |
| E2 | 完整编辑实例（name/描述/repo/binary/args/fields） | 编辑对话框保存后写回配置，字段值迁移/补齐默认 | ☑ |
| E3 | 删除实例（含二进制/日志/字段值清理） | 删除后配置与磁盘数据一并清理 | ☑ |
| E4 | 应用内日志查看（stdout/stderr 尾段） | 管理/批量页可打开日志模态并刷新 | ☑ |

## 阶段 F — 内嵌 Web 管理界面（Tauri + egui 双端，不单独打包）

> 目标：Tauri 与 egui 两进程内嵌同一 web 服务（`web-server` crate），浏览器打开 `http://127.0.0.1:端口` 即得完整管理界面；Tauri 窗口与浏览器共用同一份 SPA（单一前端）。egui 加「Web 管理」开关控制启停。

### F-1 骨架（本期）

| # | 任务 | 验收标准 | 状态 |
|---|---|---|---|
| F1 | 新增 `web-server/` workspace crate（依赖 shared）：axum + WebSocket + 静态资源服务；状态用 `Arc<Mutex<ShellManager>>`；`/api/*` 命令镜像 tauri 既有命令（起步：列程序/本地状态/启停/读日志） | 独立 lib 可起服务，egui 与 tauri 皆可依赖 | ☑ |
| F2 | SPA 单一起源 `frontend/`：收纳 app-tauri 的 HTML/CSS/JS；渲染逻辑复用现有 main.js，调用层统一为 fetch/WebSocket（`api.js`），**零 `window.__TAURI__` invoke 依赖** | 同一份前端在浏览器与 Tauri 窗口中行为一致 | ☑ |
| F3 | egui 内嵌：侧栏/设置「Web 管理」开关（**默认关闭**）；开启→随机高位空闲端口起服务→显示 URL +「在浏览器打开」；`self.manager` 改造为 `Arc<Mutex<ShellManager>>` 共享 | 开关可启停服务；本机浏览器可打开管理页 | ☑ |
| F4 | tauri 内嵌：启动即起服务（绑定 127.0.0.1），窗口加载改为主机 localhost=同一 SPA；托盘加「在浏览器打开」；原生能力（reveal/文件对话框）走后移的服务端端点（`/api/native/*`），不做 invoke fallback | 桌面窗口与浏览器访问同一前端、同一服务 | ☑ |
| F5 | 功能对齐：浏览器可达完整程序管理（列表/启停/日志/编辑/导入；v1 沿用轮询 ≈ 现有 3s） | e2e 手测：浏览器启停一次程序、日志滚动正常 | ☐ |

### F-2 安全与远程

| # | 任务 | 验收标准 | 状态 |
|---|---|---|---|
| F6 | 鉴权：启动时生成随机 token/一次性 URL 展示；`--lan` 或 UI「允许局域网」显式开启才绑 0.0.0.0 | 非本机访问必须带 token，默认 loopback 免 token | ☐ |
| F7 | 端口/绑定可配置（shell.json 设置或 UI），默认随机高位（bind 0，OS 分配）；支持固定端口 | 配置生效、冲突提示 | ☐ |
| F8 | 模板浏览/导入、批量管理、设置页全量迁移到 HTTP/WS 通道，与桌面端功能对齐 | 功能清单逐项比对无缺 | ☐ |
| F9 | 远程「binary 路径选择」策略落地：本机窗口用服务端原生 pick（rfd）；远程浏览器先支持手填，上传模式二期 | 本机原生选路径、远程可手填 | ☐ |

### F-3 体验优化（事件推送）

| # | 任务 | 验收标准 | 状态 |
|---|---|---|---|
| F10 | shared 增加轻量事件订阅（状态/日志变更回调）→ WS 推送到浏览器；egui/tauri 桌面端也改用事件替换轮询 | 外部 kill 状态 <1s 到达页面，无需轮询 | ☐ |
| F11 | 大日志增量推送（tail diff），不整页重传 | 长运行日志前端不卡顿 | ☐ |
| F12 | 移动端响应式布局，手机浏览器可启停/查看 | iOS/Android 浏览器可用 | ☐ |
| F13 | 可加一个可选的无窗口 CLI（us-web binary，只起服务不弹窗） | 给"树莓派/NAS/无桌面"场景 | ☐ |

## 执行顺序建议

A（1→7）→ B1-B3 → B4/B7（B8）→ B9 → C1/C3 → C4/C2/C5 → C6 → D1-D3 → F-1（F1→F2→F3/F4→F5）→ F-2 → F-3

## 关键决策记录

- **前端**：egui 与 Tauri 双实现，核心逻辑全部在 `shared/` 复用
- **主前端**：Tauri 为主界面（模板浏览/搜索体验 + 前端迭代快），egui 为轻量备用（托盘常驻场景）
- **模板源**：两级结构（清单 + 模板惰性拉取），本地缓存可离线回退
- **信任模型**：远程模板会引导下载并执行第三方二进制；默认安全字段值 + 来源记录 + 可选 sha256/签名
- **Web 界面**：内嵌进 Tauri 与 egui 两进程、不单独打包；SPA 单一起源 `frontend/`，浏览器与 Tauri 窗口共享同一前端、同一服务
- **Web 能力模型**：前端零 `window.__TAURI__` 依赖；原生能力（文件对话框/Finder reveal）后移为服务端端点（`/api/native/pick`、`/api/native/reveal`，rfd/`open` 在服务端执行）；`/api/capabilities` 声明可用能力，本机窗口原生体验、远程浏览器自动降级（手填/input file）
- **Web 安全默认**：绑定 127.0.0.1 + 随机高位端口；LAN/token 需显式开启；egui 开关默认关，tauri 服务 loopback 随主界面常启