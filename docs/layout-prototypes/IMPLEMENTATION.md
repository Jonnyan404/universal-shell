# 主界面布局重构 · 实施进度（交接文档）

> 目标：把「200px 侧栏平铺列表 + 单一详情区」换成 **表格 + 检查器**（B+D 融合），模板库换成 **分组卡片**（A），移动端换成 **两行列表 + 全屏二级页**。
> 设计与断点依据：[`DECISION.md`](DECISION.md)；可交互参考：[`06-merged-bd.html`](06-merged-bd.html)、[`04-master-detail.html`](04-master-detail.html)。
> 状态：**已完成并通过四档验证**（2026-11-24）。截图见 [`shots/`](shots/)。

---

## 1. 已定方案（不再讨论，直接照做）

| 区域 | 形态 | 说明 |
|---|---|---|
| 桌面 · 程序 | 左表格 + 右检查器（常驻 400px，<1280 降为 360px，<1100 变抽屉） | 检查器**按 D 方案重写**：头像 + 标题（状态胶囊同行）+ id·分类行 + 动作条 + **五个标签页**（概览/配置/调度/日志/更新）；概览页按 D 的五块排：KPI 四宫格 → 运行配置 → 依赖与关联 → 最近事件 → 日志卡 |
| 桌面 · 侧栏 | 52px 图标窄栏 | 程序 / 模板库 / 日志 / 设置 / 主题 / 语言 / GitHub |
| 桌面 · 模板库 | 分类分组卡片，组头可折叠 + 计数 | 导入动作保留原流程 |
| 移动端 · 列表 | 两行列表 + 分类小标题 | 旧卡片流下线 |
| 移动端 · 详情 | 全屏二级页（复用 `#manage-view`） | 返回回列表 |
| 不做 | 视图切换器、看板、健康矩阵 | 见 DECISION.md 第 4 节 |

**不动的东西**：`shared/` 的数据结构、所有 `invoke` 命令名、导入/更新/调度/日志的业务逻辑。这次只改「怎么摆」和「怎么渲染列表」。

---

## 2. 进度

- [x] **P0 数据补列**：`web-server/src/rpc.rs` 的 `workspace_list` 增加 `category` / `port` / `autostart`（程序侧）；
      服务侧补 `category: null` / `port: ready_port` / `autostart: enabled`。
- [x] **P1 结构**：`frontend/index.html` 侧栏改 `.iconrail`；`manage-view` 内加工具条 + 表格容器 + `aside.wb-inspector`；移动端加 `#mob-q` / `#mob-chips` / `#mob-cards`。
- [x] **P2 表格**：`frontend/workbench.js` 的 `renderList` 渲染 `<tr>`；表头排序、行高两档、批量条、自启开关、右键菜单。
- [x] **P3 样式**：`frontend/styles.css` 末尾追加「表格 + 检查器」样式块与 1280 / 1180 / 1100 / 720 四档断点。
- [x] **P4 主逻辑适配**：`frontend/main.js` 去掉对 `.sidebar` / `#program-tabs` / `#collapse-btn` / `#library-link` / `#log-center-link` 的硬依赖。
- [x] **P5 移动端**：`renderMobCards` 改两行列表 + 分组；详情全屏；详情页补回状态条与「下载」。
- [x] **P6 模板库**：`renderLibrary` 改分组卡片，卡片逻辑抽成 `libCard()`。
- [x] **P7 验证**：`us-web` 隔离数据目录（40 个程序）起服务，headless Chrome 走 DevTools 设备模拟截图 1440 / 1280 / 1100 / 390 四档 + 深色 + 模板库 + 二级页。
- [x] **P8 补列**：表格加 **运行时长** 与 **操作** 两列。
      `shared/src/runner.rs` 用 `started: BTreeMap<String, SystemTime>` 记启动时刻（退出/停止时清），
      暴露 `uptime_secs()`；`workspace_list` 每项带 `uptime`（系统调度托管 / 手工服务给 `null`，界面显示 `—`）。
      操作列三个按钮：▶/■ 启停、↻ 重启、⋯ 右键菜单；走 `quickAct()`，启动/重启会先 `get_values` 带字段值，
      否则模板里的 `{port}` / `{token}` 占位符会被渲染成空串。
- [x] **P9 检查器按 D 重写**：`aside.wb-inspector` 内改成
      `d-head(avatar + h1[名字 + 状态胶囊] + d-sub[id·分类] + 下载/关闭)` → `d-actions` → `d-tabs` → `d-body`。
      `#statusbar` / `#prog-title` / `#prog-sub` / `#status-chips` / `#actions` / `#dl-btn` 这些 id 一个没删，
      `main.js` 的写入点照旧可用。
- [x] **P10 后端补指标与事件**（D 概览页要的两张卡以前根本没数据源）：
      - `shared/src/insight.rs`：内存采样环形缓冲（24 点 / 5 秒一点）+ 事件环形缓冲（12 条），
        事件持久化到 `data_dir/insight.json`，跨重启还在；`rss_kb(pid)` 分平台实现
        （Linux `/proc/<pid>/statm`、macOS `ps -o rss=`、Windows `tasklist /FO CSV`）。
      - `Runner` 记 `exits`：`is_running` 回收子进程时存下退出码，`take_exit()` 交给 sweep，
        **主动 stop 不算退出**（stop 时先清掉），所以时间线上「进程退出（code 1）」只代表真·自己挂了。
      - `ShellManager` 持有 `insight`，`start/stop/sweep/install/set_autostart/set_supervisor` 各落一条事件；
        `sample_memory()` 由 web-server 的 `us-mem` 线程每 5 秒调一次。
      - `workspace_list` 每项多 `mem_kb`（列表只需要这一个数）；新增 `program_insight` RPC 返回
        `mem_hist` + `events`——曲线和事件比列表项重得多，全塞进列表会把 payload 撑到几百 KB。
- [x] **P11 日志窗口对齐 D**：日志从「检查器底部常驻条」搬进 **日志标签页**（D 就是这么放的），
      并新增「日志」tab；样式换成 D 的深色终端：`#0f172a` 底、`#1e293b` logbar、等宽 11.5px，
      时间戳灰、INFO 绿、WARN 黄、ERROR 红（`logLineEl` 里做 token 化，只染已知形态，绝不丢字符）。
      概览页底部另有一张 148px 的日志卡，读同一份行缓存（`usLogTail`），点「展开」跳到日志页。
- [x] **P12 模板库视觉重做**（信息架构仍是方案 A 的分类分组，视觉换成 D 的设计语言）：
      - 结构：`#library-view > .lib-shell`（页头 `.lib-hero` + 工具条 `.lib-bar` + 列表 + 分页），
        去掉原来那层 `.panel`；页头放「模板库」标题与源/模板统计，右上角是「本地模板 / 导入本地模板 / 管理源（齿轮图标）」。
      - 工具条：带放大镜的搜索框（260px，focus 蓝环）、去原生外观的源下拉（自绘 chevron）、「刷新」按钮、
        右侧补「上次拉取 …」——离线时用户能一眼看出列表来自缓存。
      - 分组头：`position: sticky` + 半透明毛玻璃底，顺序是 chevron → 分类色点 → 分类名 → 数量胶囊 → 渐隐细线；
        折叠状态存 `libCollapsed`，箭头用 CSS `rotate` 而不是 ▼/▶ 字符。
      - 卡片：30px 分类色头像块 + 名称/仓库两行 + 右上动作按钮，描述固定两行（`min-height: 3.2em` 保证同行等高），
        底部是分类标签胶囊 + 「● 本地」绿点。
      - 双色变量：`--cat-color` 按**分类**取色（标签、分组点，同组统一），`--tpl-color` 按**模板 id** 取色
        （头像、hover 描边）。只用一个色会导致同组卡片一排全蓝，没有区分度。
      - 按钮三态：未导入 = `btn primary`（蓝实心）、已导入可更新 = `btn`（描边）、已是最新 = `btn ghost`（弱化但保留描边，
        否则深色下看不见）。全部 30px 高 / 8px 圆角，与 D 方案一致。
      - 坑：`<select>` 的固有宽度按最长 option 算，移动端必须 `flex: 1 1 0; min-width: 0`，
        否则要么撑出屏幕、要么把「刷新」挤到下一行。
- [x] **P13 详情页与设计稿逐项对齐**（四页 25 条差异，逐条核对 `06-merged-bd.html` 后一次做完）：
      - **配置页**
        - 字段标签带原始 key：`访问令牌 <code>token</code>`。label 给人看，key 是 `shell.json` 里真正存的键，
          排查配置时要能对上号；两者相同时不重复。
        - 区块头补「编辑」按钮 → `openEditModal(current)`。它进的是**程序定义**（id / 名称 / args / 模板字段），
          不是下面那些即时保存的字段值；之前这个入口只藏在侧栏右键菜单里。
        - 敏感值打码：`SECRET_ENV_RE`（TOKEN / SECRET / PASSWORD / APIKEY / _KEY / AUTH / COOKIE / SESSION）
          命中即打码。env 显示成 `••••••` + 眼睛按钮（展开状态记在 `revealedEnv`，按 `programId::key` 存）；
          **字段本身改成 `input[type=password]`**——env 都打码了、字段里却明文躺着同一个 token，打码就白做了。
        - 自动保存反馈：`#cfg-save-hint` 在「字段」标题里闪一下绿色「已保存」（1.4s 淡出），
          失败常驻红色。没有反馈时用户改完不知道存没存进去，只会反复改。
        - env 空态不再隐藏整块，显示「无环境变量（当前字段没有派生任何 env）」。隐藏会让人以为程序没有 env，
          而实际可能是字段没填全、派生结果为空——那是要排查的信息。
      - **概览页**
        - KPI 顺序改为 运行时长 → 内存 → 版本 → 监听（「现在活不活」比「绑在哪」更该先看到）。
        - 依赖与关联补**导入信息**：`imported_at`（`YYYY-MM-DD HH:mm`）/ `template_source` / `repo`，
          用 `.kv-sub` 接在 chips 下面，不另起一张卡。`template_source` 常是整条
          `raw.githubusercontent.com` URL，只留末尾两段（`…/registry/ech-tunnel`），完整值放 `title`。
        - 删掉「说明」块（设计稿没有；描述已经在头部 `#prog-desc`）。「最近事件」保留。
        - 日志卡补「跟随」按钮（贴底 + 触发一次刷新），与「展开」并列。
      - **调度页**：底部补「生成的 launchd / systemd / schtasks 单元」深色代码块。
        数据来自新 RPC `program_unit_preview` → `ShellManager::unit_preview()`，
        直接调 `us-service` 的 `plist` / `unit` / `xml`，**和真正写盘的是同一份渲染**，不是前端拼的。
        竞态用自增 `unitSeq` 丢弃过期响应。标题不换行、路径省略号截断；代码块 `pre-wrap` 折行
        （检查器只有 400px 宽，横向滚动等于每行只能看前三分之一）。
      - **更新页**：补「操作」行——有新版给「应用更新」按钮（复用 `installProgram`），
        没新版显示绿色「已是最新」，没装过显示「未安装」；下方黄色提示条「更新保留已填字段，仅替换二进制并重启」
        仅在真有新版时出现。
      - **顺带修的两个真 bug**
        - `switchCurrent()` 原来是「先 `await get_values` 再渲染」：没外网时 registry 请求占满浏览器 6 条连接池，
          RPC 排队十几秒，期间详情区挂着**上一个程序**的字段——点了 A 显示 B 的配置，最坏的正确性问题。
          改成先用空值渲染骨架，值到了再回填，并用 `wanted` 比对丢弃过期响应。
          `api.js` 的 `invoke` 同时加 20s 默认超时（慢操作按调用点显式放宽：install 30min、清单/模板 2min、
          start 60s、restart 90s、batch_status 90s），否则一次挂住的请求会让 `await` 永远不返回。
        - `.seg button.on` 写死 `color: #cfe2ff`（浅蓝）——浅色主题下是白底白字，「系统调度 / 壳托管」
          分段控件里**选中项直接看不见**。改成 `color-mix` 语义色。`.wb-tag.sys` 同病同治。
        另外 `input[type=password]` 没被 `input[type=text]` 的样式覆盖，深色主题下留了浏览器默认白底，
        四处选择器都补上 `[type="password"]`。

- [x] **P14 日志不再走管道（EPIPE 根治）**：`shared/src/logpump.rs` 新增「文件 + 合并泵」。
      子进程 stdout/stderr 直接 `append` 到 `{id}.stdout` / `{id}.stderr`（`Stdio::from(File)`，
      不经过壳），壳里一个泵线程每 120ms 读新增字节，把 stderr 逐行加 `\x1F` 前缀后合并进
      `{id}.log`，再广播 `LogWritten`。**前缀染色与实时推送全部保留**，代价只是两路之间
      没有严格时序（旧实现两个线程抢同一把写锁，本来也没有）。偏移落盘 `{id}.log.off`，
      壳重启后从上次位置继续，壳死掉那段的输出会补进合并日志；泵不依赖子进程句柄，
      所以孤儿进程（外部接管）也能继续采集。细节与验证见 `docs/process-lifecycle.md` 第 7 节。

- [x] **P15 详情页收尾 + 日志中心重写**：
      - **配置页「原始 JSON」**（设计稿 `04-master-detail.html` 里本来就有，合并稿砍掉了，之前漏做）。
        新增 RPC `program_raw`：**直接读 `shell.json` 里这一项**再 `to_string_pretty`，不是把内存里的
        `Program` 序列化——手改过文件、多写了壳不认识的字段时，内存态会把这些悄悄丢掉，
        那就不是「原始」了。深色代码块 + key 蓝/值绿高亮 + 复制（复制永远给原文）。
        敏感 env / 字段默认值默认打码，点眼睛展开；否则上面 env 列表打着码、下面原文明文躺着同一个
        token，上面的打码就是自欺欺人。
      - **日志标题显示真实文件名**：详情页日志页、概览页日志卡、日志中心都改成 `{id}.log` / `shell.log`，
        不再是泛泛的「程序日志」。`get_logs` 新增返回 `path`，壳日志走新 RPC `log_path`
        （`--config` 换过目录时猜不得）。
      - **点空白关详情**：宽屏给检查器加了 `wb-insp-closed`（`✕` 按钮之前 `display:none`，宽屏根本
        没有关闭入口），窄屏抽屉加遮罩层；三条路径都能关——`✕`、点表格空白/遮罩、`Esc`。
        点任意行会把状态摘掉，不会卡死在「只有列表」。
      - **「打开网站」**：从动作区右侧一个 `↗` 图标提升为带文字的主按钮（有站点才出现）；
        列表行操作列和 `⋯` 菜单也补了入口。`webUrl()` 修了三处：只认 `host/bind/addr/url` 字段
        （不从「有端口」反推——ech-tunnel 的 listen 是代理口，拼出 `http://127.0.0.1:1080` 只会打开
        一个连不上的页面）；`0.0.0.0` 归一成 `127.0.0.1`；**非当前程序不用 `values`**（否则 A 的 host
        会拼上 B 的 port）。点击后先给「正在打开」提示——首屏 RPC 排队时 `open_url` 可能十几秒才轮到，
        期间按钮毫无动静，用户只会认为按钮坏了。
      - **日志中心重写**（保留，收窄定位为「诊断台」）：详情页日志 tab 是「瞥一眼当前程序」，
        日志中心负责它做不到的两件事——**壳自身日志**（启动失败 / RPC 报错 / 调度同步只写这里，
        没有第二个入口）和**多程序横向切换对比**。改动：源列表分组（壳日志 / 程序日志，程序按
        运行中优先排）、**「只看错误」**（stderr 行 + ERROR/WARN/PANIC 字样）、**增量渲染**
        （原来每次 2s 轮询把最多 3000 行 DOM 推倒重来，长日志肉眼可见地卡；现在只在视图键变化
        或文本被截头时重建，否则只追加新增的完整行）、头部显示当前源**实际文件名 + 行数/匹配数**、
        清空支持程序日志（新 RPC `clear_program_log`，之前只有「停止程序」会顺带清）。

---

## 3. 下一步（接手从这里开始）

主线已收口，剩下的是**真机/真实数据下的补充验证**和几个可选项：

1. **真实数据跑一遍**：隔离目录里全是「未启用」状态，运行中/异常/有更新三种状态徽标没在真实场景里看过。建议在真实数据目录里起一次，重点看 `wb-pill.running` / `.error` / `.wb-upd` 的观感。
2. **移动端真机**：390 是模拟器宽度，iOS Safari 的 `env(safe-area-inset-bottom)` 与底部导航叠压需要真机确认。
3. **可选优化**：表格目前没有「列显隐菜单」（原型 02 有），窄屏靠断点硬砍列；如果后面列还会增加，再补一个列开关。
4. **可选优化**：`#wb-table` 没有虚拟滚动，40 条无压力；如果目标场景是 500+ 程序再考虑。
5. **指标口径**：内存是 RSS（常驻物理内存），不是 PSS/虚拟内存；5 秒一个点、只留 2 分钟，
   目的是看趋势不是做监控。要接 Prometheus 之类的话另说。
6. **Windows 的内存采样**走 `tasklist /FO CSV`，比 `/proc` 慢一个数量级（每次派生进程）。
   如果 Windows 上程序数量上去了，应该换成 `GetProcessMemoryInfo`（需要加 `windows` crate）。

### 关键约束（踩过就知道疼）

- `main.js` 的 `renderSidebar()` 与 `renderMobCards()` 在 `window.usWorkbench` 存在时会**直接委托**给工作台，所以列表渲染只需改 `workbench.js`。
- `reloadPrograms()` 是「程序列表变了」的唯一入口，同时刷 `programs`/`statuses` 和 `st.items`；批量删除、切换调度归属之后必须走它。
- `st.items` 与 `main.js` 的 `programs/statuses` 是**两份缓存**，任何一边改了都要刷新另一边（历史 bug 见 `.workbuddy-ai/memory/2026-11-23.md`「侧栏导入后不刷新」）。
- `#wb-all` 的 `indeterminate` 状态依赖 `visible()` 的顺序，排序后要重新算（`renderList` 里已经每帧重算）。
- 移动端 `isMobile()` 断点是 `max-width:720px`；`mobGotoDetail()` 依赖 `body.mob-detail` 这个 class 来全屏显示 `#manage-view`。
- **前端资源是编译期内嵌的**（`web-server/src/lib.rs` 里 `include_str!`）。改完 `frontend/**` 必须 `cargo build -p app-cli` 再起服务，否则看到的还是旧界面。语言包同理。
- `select(id, openDrawer)` 的第二个参数只有「用户点行」才传 `true`。首屏自动选中第一条时不要弹抽屉，否则 1100 宽一进页面就被抽屉挡住半张表。
- 表格用 `table-layout: fixed`，行尾有一个空的 `.wb-fill` 列吃剩余宽度。**加列时别忘了它**，否则多出来的空间会全塞给名称列。
- **详情头部只能有一个写入方**。现在由 `workbench.js` 的 `renderInspHead()` 统一画，
  `main.js` 的 `renderHeader()` / `renderChips()` / `switchView("manage")` 都转调它。
  之前两边各写一半的后果：副标题被 `renderHeader` 冲成空、repo 链接插两遍、
  状态胶囊被 5 秒轮询在「未启用 / 未安装」之间来回刷。
- `renderInspHead(it)` 收进来的可能是 `main.js` 的 `Program`（没有 `category` / `state` / `repo`），
  函数内部会先用 `st.items` 里的同 id 项补全再用，别直接读 `it.state`。
- `boot()` 里 `select(items[0].id)` 必须判 `!selectedId()`：否则用户手快在 boot 完成前点了某行，会被抢回第一条。
- `body` 一定要 `margin: 0`。默认 8px + `height:100vh` + `overflow:hidden` 会把整个 app 往右下顶 8px，
  检查器底部的日志条刚好被切掉一截（看起来像日志条自己溢出了）。
- 移动端 `select()` 要**先 `mobGotoDetail()` 再 `await switchCurrent()`**。`switchCurrent` 里串着
  `get_values` / 状态刷新 / 版本检查三个 RPC，慢网下要好几秒，顺序反了用户点了行会以为没反应。
- 本机没有外网时 `template_status` 会去拉 GitHub registry 并超时，**RPC 会排队到 2~3 秒**。
  截图脚本的 `CLICK_WAIT` 因此要给到 5~6 秒，否则截到的是「详情还没渲染完」的中间态，
  很容易误判成布局 bug。
- **事件只存 `kind`，不存文案**。事件要落 `insight.json` 跨重启保留，存了中文就再也切不成英文；
  界面按 `kind` + `pid` / `code` 现拼（`workbench.js` 的 `eventText`）。
- 内存曲线和事件走**单独的 `program_insight` RPC**，不进 `workspace_list`：24 个采样点 × 40 个程序
  会把列表 payload 撑到几百 KB，而列表只需要一个 `mem_kb`。
- 日志搬进标签页之后，`refreshManageLog()` 在面板隐藏时高度为 0、滚动位置也是旧的，
  **切到日志页要再补一次**（`showTab('log')` 里已经调了）。
- 终端 `.term-body` 明暗主题都保持深色（`#0f172a`），不要改成 `var(--panel)`：
  日志窗是终端，跟着面板变白就不像终端了。

---

## 4. 验证方式

```bash
# 起一个隔离数据目录的 web 服务（不动真实配置）
cargo build -p app-cli                      # 前端是 include_str!，必须重编
HOME=/tmp/us-layout-home ./target/debug/us-web --port 18080
# 浏览器打开 http://127.0.0.1:18080  （本机 loopback 免 token）
```

截图**不要**用 `--window-size`：headless Chrome 有 500px 最小窗口宽度，390 的档位会按 500 排版再裁剪，右侧内容被切掉看起来像 bug。要走 DevTools 的设备模拟（`Emulation.setDeviceMetricsOverride`），脚本见 `/tmp/shots.mjs`（临时文件，逻辑很短，需要时重写一份即可）：

```js
await send("Emulation.setDeviceMetricsOverride", { width: 390, height: 844, deviceScaleFactor: 2, mobile: true });
await send("Page.navigate", { url });
// 等 #wb-body tr / .mob-row 出现，再 Page.captureScreenshot
```

深色主题要显式设 `document.documentElement.dataset.theme = 'dark'`，光靠 `Emulation.setEmulatedMedia` 不管用（主题走的是 `data-theme` 属性，不是 media query）。

### 验收点（已全部通过）

1. **1440**：表格铺满左侧、检查器常驻 400px；九列齐全（状态/名称/分类/版本/端口/托管/自启/**运行时长**/**操作**）；
   表头点「名称」可升/降序；勾 3 行出现批量条且计数 `3 / 40`；点行检查器标题、头像、胶囊、动作条同步刷新；
   自启开关可点；密度按钮在「紧凑 / 宽松」间切换；运行中的程序 `运行时长` 显示 `刚刚 → 1分 → 2分`，操作列变成 ■/↻/⋯。
   概览页四张 KPI 里内存是真采样（起一个会吃内存的测试进程，曲线从 35MB 一路涨到 200MB+），
   「最近事件」跟着启停出现 `由壳启动，pid …`，点「展开」跳到日志页。
2. **1280**：分类列隐藏，检查器降到 360px，其余列不挤（`scrollWidth == clientWidth`）。
3. **1100**：检查器默认收成抽屉，表格占满；点行抽屉滑出并带关闭按钮；端口列、托管列隐藏。
4. **390**：搜索框 + 筛选 chips + 两行列表（分组小标题 / 头像 / 名称 / `:端口 · v版本` / 状态点 / 箭头）；
   点行**立刻**进全屏二级页（不等 RPC），顶部返回栏 + 状态胶囊 + 下载 + 动作条 + 五个标签页都在，底部导航可用；
   日志页是铺满的深色终端。
5. **模板库**：按分类分组，组头三角可折叠带计数，卡片栅格 4 列不溢出，导入按钮在卡片内换行不裁切。
6. **深色**：表格、徽标、开关、表单标签、KPI 卡片、事件时间线对比度正常；终端本来就深色，两种主题一致。
7. **详情页四页（P13）**：配置页字段标签带 key（`访问令牌 token`）、区块头「编辑」可打开程序定义弹窗、
   敏感 env 显示 `••••••` 且眼睛可展开、改字段后标题闪「已保存」；概览页 KPI 顺序 运行时长→内存→版本→监听、
   依赖与关联出现「导入时间 / 模板来源 / 仓库」、无「说明」块、日志卡有「跟随」；调度页底部出现
   「生成的 launchd 单元」，内容与 `us-service::launchd::plist()` 的输出一致（同一份渲染，不是前端拼的）；更新页有「操作」行。
   深色主题下 password 输入框是深底（不是浏览器默认白底），`.seg` 选中项清晰可见。
   390 宽下配置页的字段与 env 行全部在屏内。

搜索过滤时计数显示 `命中 / 总数`（如 `8 / 40`），不过滤时只显示总数。

---

## 5. 回滚

本次改动集中在 8 个源文件（`index.html` / `workbench.js` / `styles.css` / `main.js` / `api.js` / `rpc.rs` / `runner.rs` / `shell_manager.rs`）
+ 新增 `shared/src/insight.rs` + `web-server/src/lib.rs` 的采样线程 + 两个语言包 + `docs/`。
若需回退：`git revert <commit>`；`rpc.rs` 的新增字段是**纯增量**，旧前端会忽略，不影响兼容。
`insight.json` 是新增的数据文件，删掉不影响运行（只是事件时间线清空）。

---

## 6. 本轮改动清单（给 review 用）

| 文件 | 改了什么 |
|---|---|
| `shared/src/logpump.rs` | **新增**：子进程日志「文件 + 合并泵」——直写文件不走管道（根治 EPIPE 自杀），stderr 行加 `\x1F` 前缀后合并，偏移落盘支持壳重启续采；4 个单测 |
| `shared/src/insight.rs` | **新增**：内存采样环形缓冲 + 事件环形缓冲（落盘 `insight.json`）+ 跨平台 `rss_kb(pid)` |
| `shared/src/runner.rs` | `Runner.started` 记录启动时刻（`uptime_secs()`）、`exits` 记录退出码（`take_exit()`）；**P14 删掉两个管道读线程（`copy_stream_lines` / `trim_program_log`），`Stdio::piped()` 改 `Stdio::from(File)`，`stop`/`is_running` 收尾时 `logpump::unwatch`** |
| `shared/src/shell_manager.rs` | 持有 `insight`；start/stop/sweep 落事件；`sample_memory()`；删程序时 `forget()`；**P13 抽出 `derive_service()` + 新增 `unit_preview()`**；**P14 `reconcile_leases` 接管时 `refresh_live()` + `logpump::watch(resume=true)`，`clear_log` 连源文件一起清，`working_dir` 空串按 `.` 处理** |
| `shared/src/lease.rs` | **P14 新增 `cmd_args`（命令行去掉 argv[0]）+ `pid_cmd_args()` + `refresh_identity()` / `refresh_live()`**：启动器再 exec 会换掉进程名（`python3` → `Python`），进程名对不上时用参数认人，否则孤儿进程停不掉 |
| `web-server/src/lib.rs` | 新增 `us-mem` 采样线程（5 秒一次） |
| `web-server/src/rpc.rs` | **P15 新增 `program_raw` / `log_path` / `clear_program_log`；`get_logs` 返回 `path`**；`workspace_list` 每项多 `category` / `port` / `autostart` / **`uptime`** / **`mem_kb`** / **`external_pid`** / **`imported_at`** / **`template_source`** / **`repo`**；新增 **`program_insight`**、**`program_unit_preview`** RPC；安装成功 / 自启 / 调度切换各记一条事件 |
| `frontend/index.html` | `.sidebar` → `.iconrail`；`manage-view` 重排成「工具条 + 表格 + 检查器」；检查器按 D 重写成 `d-head / d-actions / d-tabs / d-body`，**日志搬进「日志」标签页**；移动端加搜索/筛选；**模板库换成 `.lib-shell` + `.lib-hero` + `.lib-bar` 结构（P12）**；**P13 配置页加「编辑」+「已保存」、env 块常显、更新页加「操作」行与提示条** |
| `frontend/workbench.js` | `COLS` 增 `uptime` / `actions` 两列 + `fmtUptime()` / `actHTML()` / `quickAct()`；概览页按 D 五块重写（KPI + sparkline / 运行配置 / 依赖与关联 / 最近事件 / 日志卡）；`loadInsight()`；`renderInspHead()` 统一头部；`select()` 移动端先跳转后拉数据；**P13 概览页 KPI 重排 + 导入信息 + 日志卡「跟随」、调度页 `loadUnitPreview()`、`renderUpdatePane` 二次刷 `updateUpdateCard`** |
| `frontend/styles.css` | 图标栏、工具条、表格、检查器（`.d-*` / `.kpi` / `.spark` / `.sec` / `.kv` / `.tl`）、深色终端（`.term-body` / logbar）、四档断点、移动端两行列表；**模板库全套（`.lib-hero` / `.lib-bar` / `.btn` / sticky 分组头 / `.lib-card` / 双色变量）**；`body{margin:0}` |
| `frontend/main.js` | `renderHeader` / `renderChips` / `switchView` 的头部写入全部转调 `renderInspHead`；`logLineEl` 做时间戳/级别 token 化并导出 `usLogLine` / `usLogTail`；`switchCurrent` 先赋值 `current`；boot 不再抢选中；**模板库 `libCard()` 重写 + `libCatColor()` 分类取色 + 分组头色点/`aria-expanded` + 工具条「上次拉取」（P12）**；**P13 `switchCurrent` 先渲染骨架再回填 + `flashCfgSaved()` + 字段 key/密码框/env 打码 + 更新页「操作」行** |
| `shared/locales/zh-CN.yml` / `en.yml` | 补 `wb.col_*`、`wb.ov_*`（含 mem/dep/events/log）、`ev.*` 事件文案、`wb.tab_log` 等；P13 补 `wb.cfg_saved` / `wb.env_none` / `wb.ov_import*` / `wb.upd_apply` / `wb.sched_unit*` 等，各 679 条 |

> 语言包**只改 `shared/locales/*.yml`**。`frontend/locales/*.json` 是 `web-server/build.rs` 生成的，
> 且被 `.gitignore` 忽略，直接改 JSON 下次编译就没了。

---

## 7. 截图

| 文件 | 说明 |
|---|---|
| `shots/impl-desktop-1440.png` | 桌面 1440：九列表格 + 检查器概览（运行时长 / 端口 / **内存 + 曲线** / 版本，依赖与关联，最近事件） |
| `shots/impl-desktop-1280.png` | 1280：分类列隐藏，检查器 360px |
| `shots/impl-desktop-ov-log.png` | 概览页滚到底：D 同款深色日志卡（148px，「展开」跳日志页） |
| `shots/impl-desktop-log.png` | 日志标签页：深色终端铺满检查器 |
| `shots/impl-desktop-dark.png` | 深色主题（时间线 / chips / 日志卡对比度） |
| `shots/impl-tablet-1100.png` | 1100：检查器收起，表格占满 |
| `shots/impl-tablet-1100-drawer.png` | 1100：点行后抽屉滑出 |
| `shots/impl-mobile-390.png` | 手机 390：两行列表 |
| `shots/impl-mobile-390-detail.png` | 手机 390：全屏二级页概览（含内存曲线） |
| `shots/impl-mobile-390-log.png` | 手机 390：二级页日志页（深色终端铺满） |
| `shots/impl-library-1440.png` | 模板库：分组头 + 卡片（头像色块 / 分类标签 / 三态按钮） |
| `shots/impl-library-dark.png` | 模板库深色主题 |
| `shots/impl-library-collapse.png` | 模板库：折叠 Clipboard 分组后的状态 |
| `shots/impl-library-mobile.png` | 手机 390：页头按钮换行、搜索/源下拉/刷新单行、单列卡片 |
| `shots/impl-adopted-process.png` | 外部接管态：列表琥珀色「外部接管」胶囊 + 详情页运行时长显示 `PID xxxxx`（设计见 `docs/process-lifecycle.md`） |
| `shots/impl-detail-overview.png` | 详情页概览（P13）：KPI 顺序 运行时长 → 内存 → 版本 → 监听；依赖与关联补「导入时间 / 模板来源 / 仓库」；日志卡「跟随 + 展开」 |
| `shots/impl-detail-config.png` | 详情页配置（P13）：字段标签带原始 key（`访问令牌 token`）、区块头「编辑」、敏感值打码（字段 password + env `••••••` + 眼睛）、env 空态 |
| `shots/impl-detail-sched.png` | 详情页调度（P13）：底部「生成的 launchd 单元」深色代码块，内容来自后端 `program_unit_preview`（与写盘同一份渲染） |
| `shots/impl-detail-update.png` | 详情页更新（P13）：「操作」行显示「已是最新」（真实数据，只有 clip9 装过且无版本号） |
| `shots/impl-detail-update-avail.png` | 详情页更新（P13）：**有新版时**的形态——「应用更新」按钮 + 黄色提示条（版本号是注入的，只为验证这条路径） |
| `shots/impl-detail-config-dark.png` | 深色主题的配置页（password 输入框必须走 `--panel` 背景，不能留浏览器默认白底） |
| `shots/impl-detail-config-mobile.png` | 手机 390：配置页字段与 env 行（标签 + 值 + 眼睛）全部在屏内 |
| `shots/impl-detail-config-raw.png` | 配置页「原始 JSON」（P15）：读 `shell.json` 原文，key 蓝/值绿，敏感值打码 + 眼睛 + 复制 |
| `shots/impl-log-center.png` | 日志中心（P15 重写）：源分组「壳日志 / 程序日志」+ 当前文件 `shell.log` 与行数 |
| `shots/impl-log-center-errors.png` | 日志中心「只看错误」：12 行筛成 2 行（ERROR / WARN），计数显示 `2 / 12 行` |
| `shots/impl-detail-log-stderr.png` | 日志页（P14）：`stdout` 普通色 / `stderr` 整行错误色，两路交织。日志已改成「子进程直写文件 + 壳侧合并泵」，壳被 SIGKILL 也不会 EPIPE（见 `docs/process-lifecycle.md` 第 7 节） |
