# 主界面布局重构 · 实施进度（交接文档）

> 目标：把「200px 侧栏平铺列表 + 单一详情区」换成 **表格 + 检查器**（B+D 融合），模板库换成 **分组卡片**（A），移动端换成 **两行列表 + 全屏二级页**。
> 设计与断点依据：[`DECISION.md`](DECISION.md)；可交互参考：[`06-merged-bd.html`](06-merged-bd.html)。
> 状态：**已完成并通过三档验证**（2026-11-24）。截图见 [`shots/`](shots/)。

---

## 1. 已定方案（不再讨论，直接照做）

| 区域 | 形态 | 说明 |
|---|---|---|
| 桌面 · 程序 | 左表格 + 右检查器（常驻 400px，<1280 降为 360px，<1100 变抽屉） | 检查器里放现有的 `#statusbar` + `wb-tabs` + 三个 pane + 日志 |
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
- [x] **P7 验证**：`us-web` 隔离数据目录（40 个程序）起服务，headless Chrome 走 DevTools 设备模拟截图 1440 / 1100 / 390 三档 + 深色 + 模板库 + 二级页。

---

## 3. 下一步（接手从这里开始）

主线已收口，剩下的是**真机/真实数据下的补充验证**和几个可选项：

1. **真实数据跑一遍**：隔离目录里全是「未启用」状态，运行中/异常/有更新三种状态徽标没在真实场景里看过。建议在真实数据目录里起一次，重点看 `wb-pill.running` / `.error` / `.wb-upd` 的观感。
2. **移动端真机**：390 是模拟器宽度，iOS Safari 的 `env(safe-area-inset-bottom)` 与底部导航叠压需要真机确认。
3. **可选优化**：表格目前没有「列显隐菜单」（原型 02 有），窄屏靠断点硬砍列；如果后面列还会增加，再补一个列开关。
4. **可选优化**：`#wb-table` 没有虚拟滚动，40 条无压力；如果目标场景是 500+ 程序再考虑。

### 关键约束（踩过就知道疼）

- `main.js` 的 `renderSidebar()` 与 `renderMobCards()` 在 `window.usWorkbench` 存在时会**直接委托**给工作台，所以列表渲染只需改 `workbench.js`。
- `reloadPrograms()` 是「程序列表变了」的唯一入口，同时刷 `programs`/`statuses` 和 `st.items`；批量删除、切换调度归属之后必须走它。
- `st.items` 与 `main.js` 的 `programs/statuses` 是**两份缓存**，任何一边改了都要刷新另一边（历史 bug 见 `.workbuddy-ai/memory/2026-11-23.md`「侧栏导入后不刷新」）。
- `#wb-all` 的 `indeterminate` 状态依赖 `visible()` 的顺序，排序后要重新算（`renderList` 里已经每帧重算）。
- 移动端 `isMobile()` 断点是 `max-width:720px`；`mobGotoDetail()` 依赖 `body.mob-detail` 这个 class 来全屏显示 `#manage-view`。
- **前端资源是编译期内嵌的**（`web-server/src/lib.rs` 里 `include_str!`）。改完 `frontend/**` 必须 `cargo build -p app-cli` 再起服务，否则看到的还是旧界面。语言包同理。
- `select(id, openDrawer)` 的第二个参数只有「用户点行」才传 `true`。首屏自动选中第一条时不要弹抽屉，否则 1100 宽一进页面就被抽屉挡住半张表。
- 表格用 `table-layout: fixed`，行尾有一个空的 `.wb-fill` 列吃剩余宽度。**加列时别忘了它**，否则多出来的空间会全塞给名称列。

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

1. **1440**：表格铺满左侧、检查器常驻 400px；表头点「名称」可升/降序；勾 3 行出现批量条且计数 `3 / 40`；点行检查器标题跟着变；自启开关可点；密度按钮在「紧凑 / 宽松」间切换。
2. **1100**：检查器默认收成抽屉，表格占满；点行抽屉滑出并带关闭按钮；分类列隐藏、描述列隐藏。
3. **390**：搜索框 + 筛选 chips + 两行列表（分组小标题 / 头像 / 名称 / `:端口 · v版本` / 状态点 / 箭头）；点行进全屏二级页，顶部有状态条和「下载」按钮，底部导航可用。
4. **模板库**：按分类分组，组头三角可折叠带计数，卡片栅格 4 列不溢出，导入按钮在卡片内换行不裁切。
5. **深色**：表格、徽标、开关、表单标签对比度正常（语义色已改用 `color-mix` 跟主题走）。

搜索过滤时计数显示 `命中 / 总数`（如 `8 / 40`），不过滤时只显示总数。

---

## 5. 回滚

本次改动集中在 5 个文件（`index.html` / `workbench.js` / `styles.css` / `main.js` / `rpc.rs`）+ 两个语言包 + `docs/`。
若需回退：`git revert <commit>`；`rpc.rs` 的新增字段是**纯增量**，旧前端会忽略，不影响兼容。

---

## 6. 本轮改动清单（给 review 用）

| 文件 | 改了什么 |
|---|---|
| `web-server/src/rpc.rs` | `workspace_list` 每项多 `category` / `port` / `autostart` |
| `frontend/index.html` | `.sidebar` → `.iconrail`；`manage-view` 重排成「工具条 + 表格 + 检查器」；移动端加搜索/筛选 |
| `frontend/workbench.js` | `COLS` / `hueOf` / `sortRows` / `statePill` / `rowHTML` / `renderHead` / `applyDensity` / `renderList` / `bindTable` / `bindMob`；`renderMobCards` 改两行列表；`select(id, openDrawer)` |
| `frontend/styles.css` | 末尾 +约 290 行：图标栏、工具条、表格、检查器、四档断点、移动端两行列表、模板库分组栅格；语义色改 `color-mix` |
| `frontend/main.js` | `renderSidebar` 加空值保护；导航改图标栏高亮；`renderLibrary` 改分组 + 抽出 `libCard()`；`#wb-tablewrap` 右键菜单 |
| `frontend/locales/*.json` | 补 `wb.col_*` 八列、`wb.density_*`、`lib.uncategorized`、`ui.theme`、`toast.exported` |

---

## 7. 截图

| 文件 | 说明 |
|---|---|
| `shots/impl-desktop-1440.png` | 桌面 1440：表格 + 常驻检查器 |
| `shots/impl-desktop-dark.png` | 深色主题 |
| `shots/impl-tablet-1100.png` | 1100：检查器收起，表格占满 |
| `shots/impl-tablet-1100-drawer.png` | 1100：点行后抽屉滑出 |
| `shots/impl-mobile-390.png` | 手机 390：两行列表 |
| `shots/impl-mobile-390-detail.png` | 手机 390：全屏二级页 |
| `shots/impl-library-1440.png` | 模板库分组卡片 |
