// Universal Shell 浏览器端 SPA（F-1 骨架版）
// 与桌面窗口共用同一份代码；一切能力经 api.js 走 /api/rpc。

let programs = [];
let current = null;
let values = {};
let statuses = [];
let logTimer = null;

// 视图：manage / batch / library
let view = "manage";
let registries = [];
let registryUrl = "";
let manifest = null;
let libSource = null;
let libPage = 0;
const LIB_PAGE_SIZE = 20;
let libSearchValue = "";
// 模板库分组折叠状态（只存在内存里：浏览是一次性的，不需要跨会话记住）
const libCollapsed = new Set();
const templateStatusCache = {};
const importing = new Set();
const installing = new Set();
// 「指定版本」列表的会话级缓存：programId -> {versions, error}；/ 已尝试拉取的 programId
const versionListCache = {};
const versionListLoaded = new Set();
const UPD_EXPAND_KEY = "upd-card-expanded";
// 检查更新是否进行中（批量按钮与更新卡片按钮共享同一状态标记）
let checkingUpdates = false;
let shellUpdate = null; // {current, latest_tag, release_url}
let shellChecking = false;

const el = {
  statusbar: document.querySelector("#statusbar"),
  tabs: document.querySelector("#program-tabs"),
  progTitle: document.querySelector("#prog-title"),
  progSub: document.querySelector("#prog-sub"),
  progDesc: document.querySelector("#prog-desc"),
  chips: document.querySelector("#status-chips"),
  form: document.querySelector("#field-form"),
  actions: document.querySelector("#actions"),
  servicesView: document.querySelector("#services-view"),
  manageLog: document.querySelector("#manage-log"),
  manageLogContent: document.querySelector("#manage-log-content"),
  envBody: document.querySelector("#env-body"),
  updateSettings: document.querySelector("#upd-settings"),
  updateControls: document.querySelector("#upd-controls"),
  updateCurrent: document.querySelector("#upd-current"),
  updateLatest: document.querySelector("#upd-latest"),
  updatePin: document.querySelector("#upd-pin"),
  updateToggle: document.querySelector("#upd-toggle"),
  updateCheckedAt: document.querySelector("#upd-checked-at"),
  updateCheckBtn: document.querySelector("#upd-check-btn"),
  envSection: document.querySelector("#env-section"),
  manageView: document.querySelector("#manage-view"),
  libraryView: document.querySelector("#library-view"),
  libSourceBar: document.querySelector("#lib-source-bar"),
  libFetchInfo: document.querySelector("#lib-fetch-info"),
  libStatus: document.querySelector("#lib-status"),
  libCacheDrawer: document.querySelector("#lib-cache-drawer"),
  libCacheToggle: document.querySelector("#lib-cache-toggle"),
  libSearch: document.querySelector("#lib-search"),
  libList: document.querySelector("#lib-list"),
  libPager: document.querySelector("#lib-pager"),
  logView: document.querySelector("#log-view"),
  logSources: document.querySelector("#log-sources"),
  logSourceSelect: document.querySelector("#log-source-select"),
  logContent: document.querySelector("#log-content"),
  logClearSrcBtn: document.querySelector("#log-clear-src"),
  mobShell: document.querySelector("#mob-shell"),
  mobCards: document.querySelector("#mob-cards"),
  mobBackbar: document.querySelector("#mob-backbar"),
  mobDetailTitle: document.querySelector("#mob-detail-title"),
  mobTabbar: document.querySelector("#mob-tabbar"),
};

// ---------- 国际化 ----------
const i18n = { dict: {}, effective: "zh-CN" };

async function loadLocale() {
  const loc = await invoke("get_locale");
  i18n.effective = loc.effective || "zh-CN";
  try {
    const resp = await fetch(`./locales/${i18n.effective}.json`);
    i18n.dict = (await resp.json()) || {};
  } catch {
    i18n.dict = {};
  }
  return loc;
}

function t(key, vars) {
  let s = i18n.dict[key];
  if (s === undefined) s = key;
  if (vars) for (const [k, v] of Object.entries(vars)) s = s.split(`%{${k}}`).join(String(v));
  return s;
}

function applyStaticI18n() {
  document.querySelectorAll("[data-i18n]").forEach((n) => {
    n.textContent = t(n.dataset.i18n);
  });
  document.querySelectorAll("[data-i18n-title]").forEach((n) => {
    n.title = t(n.dataset.i18nTitle);
  });
  document.querySelectorAll("[data-i18n-placeholder]").forEach((n) => {
    n.placeholder = t(n.dataset.i18nPlaceholder);
  });
}

// ---------- toast ----------
function showNotice(msg, isError) {
  const box = document.createElement("div");
  box.className = "toast" + (isError ? " error" : " ok");
  box.textContent = msg;
  document.querySelector("#toast-container").appendChild(box);
  requestAnimationFrame(() => box.classList.add("show"));
  setTimeout(() => box.remove(), 5000);
}

// 网络类错误的明确提示：这两个按钮（检查更新 / 刷新模板库）失败只会是
// 代理网络错误或普通网络错误两种。读当前代理配置，有代理 → 提示代理问题，
// 无代理 → 提示网络问题。
async function netErrorNotice(err, n) {
  let hasProxy = false;
  try {
    const p = await invoke("get_proxy");
    hasProxy = !!p.effective_http_proxy || !!p.effective_accelerate_prefix;
  } catch {}
  const key = hasProxy ? (n ? "err.net_proxy_n" : "err.net_proxy") : (n ? "err.network_n" : "err.network");
  showNotice(n ? t(key, { count: n }) : t(key), true);
  if (err) console.warn("net error detail:", err);
}

async function openExternal(url) {
  try {
    await invoke("open_url", { url });
  } catch (e) {
    window.open(url, "_blank", "noopener");
  }
}

// ---------- 侧栏 ----------
function renderSidebar(preferId) {
  // 列表由工作台统一渲染（程序 + 手工服务）；旧实现保留为兜底
  if (window.usWorkbench) return window.usWorkbench.renderList(preferId);
  if (!el.tabs) return;
  el.tabs.innerHTML = "";
  statuses.forEach((s) => {
    const it = document.createElement("div");
    it.className = "prog-item" + (current && current.id === s.id ? " active" : "");
    it.dataset.id = s.id;
    const dot = document.createElement("span");
    dot.className = "status-dot" + (s.status.running ? " on" : "");
    const ico = document.createElement("span");
    ico.className = "prog-ico";
    ico.textContent = (s.name || "?").slice(0, 1).toUpperCase();
    const info = document.createElement("span");
    info.className = "info";
    const name = document.createElement("span");
    name.className = "name";
    const nm = document.createElement("span");
    nm.className = "nm";
    nm.textContent = s.name;
    name.appendChild(nm);
    const sub = document.createElement("span");
    sub.className = "sub";
    const repo = programs.find((p) => p.id === s.id)?.repo;
    sub.textContent = s.status.running
      ? "● " + t("st.running")
      : "○ " + (s.status.installed ? t("st.stopped") : repo ? t("st.not_installed", { repo }) : t("st.not_installed_bare"));
    info.appendChild(name);
    info.appendChild(sub);
    it.appendChild(dot);
    it.appendChild(ico);
    it.appendChild(info);
    it.onclick = () => switchCurrent(s.id);
    it.oncontextmenu = (e) => {
      e.preventDefault();
      e.stopPropagation();
      const p = programs.find((x) => x.id === s.id);
      ctxMenuAt(e.clientX, e.clientY, progItemMenu(p));
    };
    attachCtxLongPress(it, (x, y) => {
      const p = programs.find((x2) => x2.id === s.id);
      ctxMenuAt(x, y, progItemMenu(p));
    });
    el.tabs.appendChild(it);
  });
  if (preferId && programs.some((p) => p.id === preferId)) {
    const target = el.tabs.querySelector(`.prog-item[data-id="${preferId}"]`);
    target && target.scrollIntoView({ block: "nearest" });
  }
}

// ---------- 侧栏右键菜单 ----------
function ctxMenuAt(x, y, items) {
  hideCtxMenu();
  const menu = document.querySelector("#ctx-menu");
  for (const it of items) {
    if (it.sep) {
      const s = document.createElement("div");
      s.className = "ctx-sep";
      menu.appendChild(s);
      continue;
    }
    const b = document.createElement("button");
    b.className = "ctx-item" + (it.danger ? " danger" : "");
    b.textContent = it.label;
    b.onclick = () => {
      hideCtxMenu();
      it.onClick();
    };
    menu.appendChild(b);
  }
  menu.hidden = false;
  const r = menu.getBoundingClientRect();
  let px = x;
  let py = y;
  if (px + r.width > window.innerWidth) px = window.innerWidth - r.width - 4;
  if (py + r.height > window.innerHeight) py = window.innerHeight - r.height - 4;
  menu.style.left = px + "px";
  menu.style.top = py + "px";
}

function hideCtxMenu() {
  const menu = document.querySelector("#ctx-menu");
  menu.hidden = true;
  menu.innerHTML = "";
}

// 侧栏/卡片条目右键菜单项（桌面右键与移动长按共用）
function progItemMenu(p) {
  return [
    { label: t("menu.copy"), onClick: () => duplicateProgram(p) },
    { label: t("act.edit"), onClick: () => openEditModal(p) },
    { sep: true },
    { label: t("act.delete"), danger: true, onClick: () => confirmAndDelete(p) },
  ];
}

// ---------- 移动端长按 = 右键菜单 ----------
// 触摸设备无右键：按压 450ms 弹出等价菜单；长按后抑制随后的 click（避免误触切换）。
let ctxLongPressTimer = null;
let ctxLongPressSuppress = false;
const TOUCH_ONLY = window.matchMedia("(hover: none)").matches;

function clearCtxLongPress() {
  if (ctxLongPressTimer) {
    clearTimeout(ctxLongPressTimer);
    ctxLongPressTimer = null;
  }
}

function attachCtxLongPress(el, openAt) {
  if (!TOUCH_ONLY) return;
  let sx = 0;
  let sy = 0;
  el.addEventListener("pointerdown", (e) => {
    if (e.pointerType !== "touch") return;
    e.stopPropagation();
    sx = e.clientX;
    sy = e.clientY;
    clearCtxLongPress();
    ctxLongPressTimer = setTimeout(() => {
      ctxLongPressTimer = null;
      ctxLongPressSuppress = true;
      openAt(sx, sy);
      if (navigator.vibrate) navigator.vibrate(10);
    }, 450);
  });
  el.addEventListener("pointermove", (e) => {
    if (ctxLongPressTimer && (Math.abs(e.clientX - sx) > 10 || Math.abs(e.clientY - sy) > 10)) {
      clearCtxLongPress();
    }
  });
  el.addEventListener("pointerup", clearCtxLongPress);
  el.addEventListener("pointercancel", clearCtxLongPress);
  el.addEventListener("pointerleave", clearCtxLongPress);
}

// 长按触发后拦截随后的 click：避免进入详情/切换程序
document.addEventListener("click", (e) => {
  if (ctxLongPressSuppress) {
    ctxLongPressSuppress = false;
    e.preventDefault();
    e.stopPropagation();
    e.stopImmediatePropagation();
  }
}, true);

document.addEventListener("click", hideCtxMenu);
window.addEventListener("resize", hideCtxMenu);

// 列表空白区右键 = 新建（行右键由工作台自己处理，不会冒泡到这里）。
const wbTableWrap = document.querySelector("#wb-tablewrap");
if (wbTableWrap) {
  wbTableWrap.addEventListener("contextmenu", (e) => {
    if (e.target.closest("tr, #ctx-menu, .modal")) return;
    e.preventDefault();
    ctxMenuAt(e.clientX, e.clientY, [{ label: t("menu.new"), onClick: () => openEditModal(null) }]);
  });
}

async function duplicateProgram(p) {
  try {
    const copy = await invoke("duplicate_program", { programId: p.id });
    showNotice(t("toast.duplicated", { name: copy.name }));
    await reloadPrograms();
    switchCurrent(copy.id);
  } catch (e) {
    showNotice(String(e), true);
  }
}

// 删除：弹窗二选一（保留数据 / 连带数据彻底删除）
let delTarget = null;
function confirmAndDelete(p) {
  delTarget = p;
  document.querySelector("#del-modal-desc").textContent = t("del.desc", {
    name: p.name,
    id: p.id,
  });
  document.querySelector("#del-modal").hidden = false;
}
function closeDelModal() {
  document.querySelector("#del-modal").hidden = true;
  delTarget = null;
}
async function applyDelete(entirely) {
  const p = delTarget;
  closeDelModal();
  if (!p) return;
  try {
    await invoke(entirely ? "delete_program_entirely" : "delete_program", { programId: p.id });
    showNotice(t(entirely ? "toast.deleted_purge" : "toast.deleted", { name: p.name }));
    await reloadPrograms();
    if (current && current.id === p.id) {
      const next = programs[0];
      if (next) switchCurrent(next.id);
      else document.querySelector("#manage-view").hidden = true;
    }
  } catch (e) {
    showNotice(String(e), true);
  }
}

async function confirmAndClearData(p) {
  if (!confirm(t("confirm.clear_data", { name: p.name }))) return;
  try {
    await invoke("clear_program_data", { programId: p.id });
    showNotice(t("toast.data_cleared", { name: p.name }));
    if (window.usWorkbench) await window.usWorkbench.load();
  } catch (e) {
    showNotice(String(e), true);
  }
}

// ---------- 主视图 ----------
function switchView(v) {
  // 批量与服务已并入工作台；这里保留映射，旧调用点不必逐个改
  if (v === "batch" || v === "services") v = "manage";
  view = v;
  el.manageView.hidden = v !== "manage";
  el.libraryView.hidden = v !== "library";
  el.logView.hidden = v !== "log";
  if (v === "manage") window.usWorkbench && window.usWorkbench.onShow();
  else window.usWorkbench && window.usWorkbench.onHide();
  const dlBtn = document.querySelector("#dl-btn");
  if (dlBtn) dlBtn.hidden = v !== "manage";
  // 更新设置条在 #manage-view 内部，跟着 manageView 一起显示/隐藏，这里不再单独控制
  if (v === "manage") {
    el.statusbar.hidden = false;
    if (current) {
      el.progTitle.textContent = current.name;
      el.progSub.innerHTML = "";
    }
    el.progDesc.hidden = true;
  } else {
    // 批量/模板库/日志中心不再显示顶部标题栏（程序名状态条只在管理页需要）
    el.statusbar.hidden = true;
    if (v === "library") {
      if (!manifest) ensureLibraryFromCache();
      else renderLibrary();
    }
  }
  if (v !== "log") stopLogCenterTailing();
  renderSidebar();
  // 图标栏高亮：程序 / 模板库 / 日志三个入口互斥
  const navActive = v === "library" ? "nav-lib" : v === "log" ? "nav-log" : "nav-prog";
  document.querySelectorAll(".iconrail .ir-btn").forEach((b) => {
    if (["nav-prog", "nav-lib", "nav-log"].includes(b.id)) b.classList.toggle("on", b.id === navActive);
  });
  if (isMobile()) mobSyncView(v);
}

async function switchCurrent(id) {
  switchView("manage");
  current = programs.find((p) => p.id === id) || null;
  if (!current) return;
  values = (await invoke("get_values", { programId: current.id }).catch(() => ({}))) || {};
  renderHeader();
  renderForm();
  renderActions();
  await refreshStatusLocal();
  await renderUpdateSettings();
  const hint = document.querySelector("#manage-log-hint");
  if (hint) hint.textContent = current.id;
  refreshManageLog();
}

function isUpdateCardExpanded(id) {
  try {
    const a = JSON.parse(localStorage.getItem(UPD_EXPAND_KEY) || "[]");
    return a.includes(id);
  } catch {
    return false;
  }
}
function setUpdateCardExpanded(id, on) {
  try {
    let a = JSON.parse(localStorage.getItem(UPD_EXPAND_KEY) || "[]");
    const i = a.indexOf(id);
    if (on && i < 0) a.push(id);
    if (!on && i >= 0) a.splice(i, 1);
    localStorage.setItem(UPD_EXPAND_KEY, JSON.stringify(a));
  } catch {}
}

// ---------- 更新设置（方案A：状态栏「更新设置」→ 展开条） ----------
// 默认折叠：管理页只有状态栏一个「更新设置」小按钮；点开才显示版本行+通道+检查更新。
// 折叠状态按程序 id 持久化到 localStorage（新程序默认折叠）。
async function renderUpdateSettings() {
  const box = el.updateSettings;
  const toggle = el.updateToggle;
  if (!box || !toggle || !current) return;
  const isHttp = !!current.http_enabled;
  const isLocal = !current.repo && !isHttp;
  if (isLocal) {
    // 本地程序：没有更新设置，按钮和折叠条一起消失
    box.hidden = true;
    toggle.hidden = true;
    return;
  }
  // 更新设置已是工作台的一个 tab，状态栏的折叠开关不再需要
  toggle.hidden = true;
  box.dataset.programId = current.id;
  const st = statuses.find((s) => s.id === current.id)?.status;
  updateUpdateCard(st);

  // 折叠状态：默认折叠（persist per program）。
  // 移动端没有状态栏、也就没有那个「展开/收起」按钮，所以一律展开，否则永远打不开。
  const expanded = isUpdateCardExpanded(current.id) || isMobile();
  box.hidden = !expanded;
  const renderToggle = () => {
    toggle.classList.toggle("open", !!expanded);
    toggle.querySelector(".upd-toggle-caret").textContent = expanded ? "▾" : "▸";
    toggle.title = expanded ? t("upd.collapse") : t("upd.expand");
  };
  renderToggle();
  toggle.onclick = () => {
    const now = isUpdateCardExpanded(current.id);
    box.hidden = now;
    toggle.classList.toggle("open", !now);
    toggle.querySelector(".upd-toggle-caret").textContent = now ? "▸" : "▾";
    toggle.title = now ? t("upd.expand") : t("upd.collapse");
    setUpdateCardExpanded(current.id, !now);
  };

  // 钉住指定版本时在版本行显示小 pill，折叠也能看到（折叠时整条隐藏，展开可见）
  if (el.updatePin) {
    const pinv = current.version_pin || "";
    el.updatePin.textContent = pinv;
    el.updatePin.hidden = !pinv;
  }

  // 检查更新按钮：与批量按钮共享 checkingUpdates 状态
  const chkBtn = el.updateCheckBtn;
  if (chkBtn) {
    chkBtn.onclick = () => checkUpdates(chkBtn);
    chkBtn.disabled = checkingUpdates;
    chkBtn.textContent = checkingUpdates ? t("dl.checking_short") : t("batch.check");
  }

  // 控件行（通道 + 指定版本）：仅 GitHub 远程源构建
  const ctrl = el.updateControls;
  if (!ctrl) return;
  ctrl.innerHTML = "";
  ctrl.hidden = isHttp || !current.repo;
  if (isHttp || !current.repo) return;

  // 更新通道：稳定版 / 预发布
  const chan = document.createElement("div");
  chan.className = "upd-row";
  const chanLabel = document.createElement("span");
  chanLabel.className = "upd-label";
  chanLabel.textContent = t("upd.channel");
  chan.appendChild(chanLabel);
  const chanGroup = document.createElement("span");
  chanGroup.className = "upd-radios";
  const effChan = current.channel === "pre" ? "pre" : "stable";
  for (const [value, label] of [["stable", t("upd.stable")], ["pre", t("upd.pre")]]) {
    const lab = document.createElement("label");
    lab.className = "upd-radio";
    const inp = document.createElement("input");
    inp.type = "radio";
    inp.name = "upd-channel";
    inp.value = value;
    inp.checked = effChan === value;
    inp.addEventListener("change", async () => {
      if (!inp.checked) return;
      try {
        const v = await invoke("set_channel", { programId: current.id, channel: value });
        Object.assign(current, { channel: v.channel, version_pin: v.version_pin });
        showNotice(t("toast.channel_set", { channel: value === "pre" ? t("upd.pre") : t("upd.stable") }));
        await renderUpdateSettings();
        await refreshStatusLocal();
      } catch (e) {
        showNotice(String(e), true);
      }
    });
    lab.appendChild(inp);
    lab.appendChild(document.createTextNode(" " + label));
    chanGroup.appendChild(lab);
  }
  chan.appendChild(chanGroup);
  ctrl.appendChild(chan);

  // 指定版本：自动(随通道) + 最近 N 个发行版。
  // 版本列表按需拉取：切到该程序/刷新时不联网，首次点击/聚焦下拉时才请求
  // （会话级缓存：同一程序只请求一次，除非用户主动再次点击刷新图标）。
  const vrow = document.createElement("div");
  vrow.className = "upd-row";
  const vLabel = document.createElement("span");
  vLabel.className = "upd-label";
  vLabel.textContent = t("upd.version");
  vrow.appendChild(vLabel);
  const sel = document.createElement("select");
  sel.className = "upd-select";
  vrow.appendChild(sel);
  const verNote = document.createElement("span");
  verNote.className = "upd-note";
  vrow.appendChild(verNote);
  const verRefresh = document.createElement("button");
  verRefresh.type = "button";
  verRefresh.className = "icon-btn upd-refresh";
  verRefresh.title = t("act.refresh");
  verRefresh.textContent = "↻";
  verRefresh.hidden = true;
  vrow.appendChild(verRefresh);
  ctrl.appendChild(vrow);

  const pin = current.version_pin || "";
  // 初始仅「自动」；已钉住时补一项当前 pin，避免一进程序就联网拉列表
  const autoOpt = document.createElement("option");
  autoOpt.value = "";
  autoOpt.textContent = t("upd.auto");
  autoOpt.selected = !pin;
  sel.appendChild(autoOpt);
  if (pin) {
    const pinOpt = document.createElement("option");
    pinOpt.value = pin;
    pinOpt.textContent = pin + " (pin)";
    pinOpt.selected = true;
    sel.appendChild(pinOpt);
  }

  // 会话级缓存
  const cached = versionListCache[current.id];
  if (cached) {
    applyVersions(cached.versions, cached.error);
  }

  const ensureVersions = async () => {
    if (versionListLoaded.has(current.id)) return;
    versionListLoaded.add(current.id);
    verRefresh.disabled = true;
    const loadOpt = document.createElement("option");
    loadOpt.value = "";
    loadOpt.textContent = t("upd.loading");
    loadOpt.selected = true;
    sel.add(loadOpt);
    try {
      const r = await invoke("list_versions", { programId: current.id, limit: 15 });
      const versions = (r && r.versions) || [];
      versionListCache[current.id] = { versions, error: "" };
      applyVersions(versions, "");
    } catch (e) {
      const err = String(e);
      versionListCache[current.id] = { versions: [], error: err };
      applyVersions([], err);
    } finally {
      verRefresh.disabled = false;
    }
  };

  function applyVersions(versions, error) {
    // 清掉「自动」之后的选项（含加载中占位），保留第一项自动
    while (sel.options.length > 1) sel.remove(1);
    if (error) {
      verNote.textContent = t("upd.list_fail", { err: error });
      verNote.classList.add("upd-error");
      const none = document.createElement("option");
      none.value = "";
      none.textContent = t("upd.none");
      none.selected = !pin;
      none.disabled = true;
      sel.appendChild(none);
    } else if (!versions.length) {
      const none = document.createElement("option");
      none.value = "";
      none.textContent = t("upd.none");
      none.selected = !pin;
      none.disabled = true;
      sel.appendChild(none);
    } else {
      for (const vv of versions) {
        const opt = document.createElement("option");
        opt.value = vv.tag;
        opt.textContent = vv.tag + (vv.prerelease ? " ⚠" : "");
        opt.selected = vv.tag === pin;
        sel.appendChild(opt);
      }
      // 钉住的版本不在最近列表内：仍补充一项标注原钉住值
      if (pin && !versions.some((x) => x.tag === pin)) {
        const opt = document.createElement("option");
        opt.value = pin;
        opt.textContent = pin + " (pin)";
        opt.selected = true;
        sel.appendChild(opt);
      }
    }
    sel.value = pin || "";
    sel.disabled = false;
    verNote.classList.remove("upd-error");
  }

  // 首次交互(点击/聚焦下拉)时才拉版本列表；之后会话内直接走缓存
  sel.addEventListener("pointerdown", ensureVersions);
  sel.addEventListener("focus", ensureVersions);
  // 手动刷新：清除缓存后再次触发拉取
  verRefresh.addEventListener("click", async () => {
    versionListLoaded.delete(current.id);
    delete versionListCache[current.id];
    await ensureVersions();
  });
  verRefresh.hidden = false;

  sel.addEventListener("change", async () => {
    try {
      const v = await invoke("set_version_pin", { programId: current.id, version: sel.value });
      Object.assign(current, { channel: v.channel, version_pin: v.version_pin });
      await renderUpdateSettings();
      await refreshStatusLocal();
    } catch (e) {
      showNotice(String(e), true);
    }
  });
}

// 刷新更新卡片的版本行（当前 → 最新）。状态每次更新时由 applyStatus 调用。
function updateUpdateCard(st) {
  if (!el.updateSettings || el.updateSettings.hidden) return;
  if (!st) st = statuses.find((s) => s.id === current?.id)?.status;
  const cur = el.updateCurrent;
  const latest = el.updateLatest;
  if (!cur || !latest) return;
  const local = st?.installed ? `v${st.local_version}` : t("st.not_installed_bare");
  cur.textContent = local;
  const newest = st?.latest_version ? `v${st.latest_version}` : "—";
  latest.textContent = newest;
  const newer =
    st?.installed &&
    st?.latest_version &&
    st?.latest_version !== st?.local_version;
  latest.classList.toggle("newer", !!newer);
  // 上次检查时间
  if (el.updateCheckedAt) {
    const at = st?.latest_checked_at;
    el.updateCheckedAt.textContent = at
      ? t("check.last_checked", { ago: timeAgo(at) })
      : t("check.not_checked");
  }
}

function renderHeader() {
  el.progTitle.textContent = current.name;
  el.progSub.textContent = "";
  const repo = (current.repo || "").trim();
  if (repo) {
    const a = document.createElement("a");
    a.className = "repo-link";
    a.title = `GitHub · ${repo}`;
    a.href = "#";
    a.innerHTML = '<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"/></svg>';
    a.onclick = (e) => {
      e.preventDefault();
      openExternal("https://github.com/" + repo);
    };
    el.progSub.appendChild(a);
  }
  el.progDesc.textContent = current.description || "";
  el.progDesc.hidden = !current.description;
}

// 字段值防抖自动保存（对齐桌面端体验）
const autosaveTimers = {};
// 系统调度接管时的「重新装载」去抖：连续输入不该把服务反复重启
const nativeApplyTimers = {};

function scheduleNativeApply(id) {
  clearTimeout(nativeApplyTimers[id]);
  nativeApplyTimers[id] = setTimeout(async () => {
    try {
      const r = await invoke("apply_supervisor", { programId: id });
      // 内容没变时后端返回 applied=false，此时什么都不做（避免无谓提示）
      if (r && r.applied) showNotice(t("svc.applied"));
    } catch (e) {
      showNotice(t("svc.apply_failed", { err: String(e) }), true);
    }
  }, 1200);
}

function scheduleAutosave() {
  if (!current) return;
  const id = current.id;
  const isNative = current.supervisor === "native";
  const snap = { ...values };
  clearTimeout(autosaveTimers[id]);
  autosaveTimers[id] = setTimeout(async () => {
    try {
      await invoke("save_values", { programId: id, values: snap });
      // 由系统调度接管的程序：字段值改了要重新派生并重装，
      // 否则系统里跑的仍是旧参数（端口、路径、token 都可能是旧的）。
      if (isNative) scheduleNativeApply(id);
    } catch (e) {
      showNotice(String(e), true);
    }
  }, 600);
}

function renderForm() {
  el.form.innerHTML = "";
  for (const f of current.fields) {
    const row = document.createElement("div");
    row.className = "field-row";
    const label = document.createElement("label");
    label.className = "field-label" + (f.required ? " required" : "");
    label.textContent = f.required ? f.label + " *" : f.label;
    row.appendChild(label);

    if (f.kind === "boolean") {
      const check = document.createElement("input");
      check.type = "checkbox";
      check.checked = (values[f.key] ?? f.default) === "true";
      check.addEventListener("change", () => {
        values[f.key] = check.checked ? "true" : "false";
        scheduleAutosave();
      });
      row.appendChild(check);
    } else {
      const input = document.createElement("input");
      input.type = "text";
      input.setAttribute("autocorrect", "off");
      input.setAttribute("spellcheck", "false");
      input.setAttribute("autocapitalize", "off");
      if (f.placeholder) input.placeholder = f.placeholder;
      input.value = values[f.key] ?? f.default;
      input.addEventListener("input", () => {
        values[f.key] = input.value;
        scheduleAutosave();
      });
      row.appendChild(input);
      // 文件/目录类型：显示「浏览」按钮，走宿主机原生选择器（本机窗口可用）
      if ((f.kind === "file" || f.kind === "directory") && globalThis.haveNativePick) {
        const br = document.createElement("button");
        br.type = "button";
        br.className = "op-btn";
        br.textContent = t("act.browse");
        br.onclick = async () => {
          try {
            const r = await invoke(f.kind === "file" ? "pick_file" : "pick_directory");
            if (r.supported && r.path) {
              values[f.key] = r.path;
              input.value = r.path;
              scheduleAutosave();
            }
          } catch (e) {
            showNotice(String(e), true);
          }
        };
        row.appendChild(br);
      }
    }
    el.form.appendChild(row);
  }
  renderEnvDisplay();
}

function renderEnvDisplay() {
  el.envSection.hidden = !current.env.length;
  el.envBody.innerHTML = "";
  for (const e of current.env) {
    const row = document.createElement("div");
    row.className = "field-row";
    const lab = document.createElement("label");
    lab.className = "field-label";
    lab.textContent = e.label || e.key;
    const val = document.createElement("input");
    val.type = "text";
    val.readOnly = true;
    let v = e.value;
    for (const [k, fv] of Object.entries(values)) v = v.split(`{${k}}`).join(String(fv));
    val.value = v;
    row.appendChild(lab);
    row.appendChild(val);
    el.envBody.appendChild(row);
  }
}

function renderActions() {
  const actions = el.actions;
  actions.innerHTML = "";
  // 选中手工服务时操作区本来就是隐藏的；此时 current 为 null，别往下走。
  if (!current) return;
  // 按钮组整体居中；启停/重启 与 图标组 之间用分隔条隔开
  const group1 = document.createElement("span");
  group1.className = "act-group";

  // 「开机启动」是唯一事实来源（program-autostart.json）：
  //   壳托管   → 壳启动时代为拉起；
  //   系统调度 → 决定派生服务是否装载到系统启动项。
  // 两种托管方式都得有这个开关，不能因为切到 native 就把它藏掉。
  const curStatus = statuses.find((s) => current && s.id === current.id)?.status;
  const autoOn = !!(curStatus && curStatus.autostart);
  const autoBtn = document.createElement("button");
  autoBtn.id = "autostart-btn";
  autoBtn.textContent = "⏻ " + t(autoOn ? "svc.autostart_disable" : "svc.autostart_enable");
  autoBtn.onclick = () => toggleAutostart(autoOn);
  group1.appendChild(autoBtn);

  const start = document.createElement("button");
  start.id = "start-btn";
  start.textContent = "▶ " + t("act.start");
  start.onclick = async () => {
    try {
      const st = await invoke("start_program", { programId: current.id, values });
      applyStatus(st);
      // 后端可能顺带改了「开机自启」（native 下启动=装载），要如实说出来
      showNotice((st && st.note) || t("toast.started"));
      refreshManageLog();
    } catch (e) {
      showNotice(String(e), true);
    }
  };
  group1.appendChild(start);

  const stop = document.createElement("button");
  stop.id = "stop-btn";
  stop.disabled = true;
  stop.textContent = "■ " + t("act.stop");
  stop.onclick = async () => {
    try {
      const st = await invoke("stop_program", { programId: current.id });
      applyStatus(st);
      showNotice(t("toast.stopped"));
      refreshManageLog();
    } catch (e) {
      showNotice(String(e), true);
    }
  };
  group1.appendChild(stop);

  const restart = document.createElement("button");
  restart.id = "restart-btn";
  restart.disabled = true;
  restart.textContent = "↻ " + t("act.restart");
  restart.onclick = async () => {
    try {
      const st = await invoke("restart_program", { programId: current.id, values });
      applyStatus(st);
      showNotice((st && st.note) || t("toast.restarted"));
      refreshManageLog();
    } catch (e) {
      showNotice(String(e), true);
    }
  };
  group1.appendChild(restart);
  // 系统调度下启停走后端 native_dispatch（直接操作系统调度器，不经过壳的
  // Runner，不会双开），所以按钮照常画完；末尾只多一个回退入口。
  if (current.supervisor === "native") {
    const back = document.createElement("button");
    back.textContent = "⏻ " + t("svc.back_to_managed");
    back.onclick = () => setSupervisor(false);
    group1.appendChild(back);
  }
  actions.appendChild(group1);

  // 下载/更新按钮已迁入状态栏（对齐 Tauri），这里只接上绑定
  const dl = document.querySelector("#dl-btn");
  if (dl) {
    dl.dataset.programId = current.id;
    dl.dataset.installed = statuses.find((s) => s.id === current.id)?.status?.installed ? "1" : "0";
    dl.onclick = () => installProgram(current.id, dl);
  }

  // 图标组（打开应用目录/复制地址/打开网站）：仅在存在时插入分隔条
  const group2 = document.createElement("span");
  group2.className = "act-group act-icons";
  const pushIcon = (b) => {
    group2.appendChild(b);
  };
  // 本地程序无壳内数据目录，不显示「打开应用目录」
  if (current.repo) {
    const dirBtn = document.createElement("button");
    dirBtn.className = "icon-btn";
    dirBtn.title = t("act.open_app_dir");
    dirBtn.textContent = "📂";
    dirBtn.onclick = async () => {
      try {
        await invoke("reveal_app_dir", { programId: current.id });
        showNotice(t("toast.opened_dir"));
      } catch (e) {
        showNotice(String(e), true);
      }
    };
    pushIcon(dirBtn);
  }
  const url = webUrl(current);
  if (url) {
    const copy = document.createElement("button");
    copy.className = "icon-btn";
    copy.title = t("act.copy_addr");
    copy.textContent = "⧉";
    copy.onclick = async () => {
      try {
        await navigator.clipboard.writeText(url);
        showNotice(t("toast.addr_copied"));
      } catch {
        showNotice(t("toast.copy_fail"), true);
      }
    };
    pushIcon(copy);
    const open = document.createElement("button");
    open.className = "icon-btn";
    open.title = t("act.open_site");
    open.textContent = "↗";
    open.onclick = () => openExternal(url);
    pushIcon(open);
  }
  if (group2.children.length) {
    const sep = document.createElement("span");
    sep.className = "act-sep";
    sep.setAttribute("aria-hidden", "true");
    actions.appendChild(sep);
    actions.appendChild(group2);
  }
}

// ---------- 状态 ----------
async function refreshStatusLocal() {
  if (!current) return;
  try {
    const st = await invoke("get_status_local", { programId: current.id });
    applyStatus(st);
  } catch (e) {
    showNotice(String(e), true);
  }
}

function applyStatus(st) {
  const idx = statuses.findIndex((s) => s.id === current.id);
  if (idx >= 0) statuses[idx].status = st;
  renderChips(st);
  // 开机启动开关的文案跟着 autostart 状态走；状态一到就重画，不能只改 disabled。
  renderActions();
  renderButtons(st);
  syncInstallBtns(current.id);
  updateUpdateCard(st);
  renderSidebar();
}

// 只切换「是否开机启动」，不碰 supervisor。壳托管写 program-autostart.json
// 由壳拉起；系统调度下后端还会同步 services.toml 的 enabled 并装载/卸载
// 系统启动项，所以两条路径都是同一个开关。
async function toggleAutostart(wasOn) {
  if (!current) return;
  try {
    await invoke("set_autostart", { programId: current.id, enabled: !wasOn });
    showNotice(t("toast.autostart_updated", { name: current.name }));
    await refreshStatusLocal();
    if (window.usWorkbench && typeof window.usWorkbench.load === "function") {
      await window.usWorkbench.load();
    }
  } catch (e) {
    showNotice(String(e), true);
  }
}

async function setSupervisor(native) {
  try {
    const r = await invoke("set_supervisor", {
      programId: current.id,
      supervisor: native ? "native" : "managed",
    });
    showNotice(r.message || "");
    await reloadPrograms();
    switchCurrent(current.id);
    // 首次切到 native 时后端会顺手打开开机自启，状态栏要跟上（列表已由
    // reloadPrograms 刷过）。
    await refreshStatusLocal();
  } catch (e) {
    showNotice(String(e), true);
  }
}

function renderChips(st) {
  el.chips.innerHTML = "";
  const mk = (text, cls) => {
    const p = document.createElement("span");
    p.className = "pill " + (cls || "");
    p.textContent = text;
    el.chips.appendChild(p);
  };
  if (st.installed) {
    mk(`v${st.local_version}`, "ok");
  } else {
    mk(t("st.not_installed_bare"));
  }
  if (st.running) mk("● " + t("st.running"), "running");
  if (st.autostart) mk("⏻ " + t("st.autostart"), "warn");
}

function renderButtons(st) {
  document.querySelectorAll("#actions button").forEach((b) => {
    if (b.id === "start-btn") b.disabled = st.running;
    if (b.id === "stop-btn") b.disabled = !st.running;
    if (b.id === "restart-btn") b.disabled = !st.running;
  });
}

// 构造程序的 Web 访问地址(如有地址/端口字段)。无地址返回 null
function webUrl(prog) {
  if (!prog) return null;
  const fieldVal = (key) => {
    const v = values[key];
    if (v && String(v).trim()) return String(v).trim();
    const f = prog.fields.find((x) => x.key === key);
    return f && f.default && String(f.default).trim() ? String(f.default).trim() : null;
  };
  const addr = fieldVal("host") ?? fieldVal("bind") ?? fieldVal("addr");
  if (!addr) return null;
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(addr)) return addr;
  const port = fieldVal("port");
  if (port && !/:\d+$/.test(addr)) return `http://${addr}:${port}`;
  return `http://${addr}`;
}

// ---------- 下载 / 安装（进度走 WS 事件总线） ----------
async function installProgram(id, btn) {
  if (installing.has(id)) return;
  installing.add(id);
  if (btn) {
    btn.disabled = true;
    btn.classList.add("dl-progress-btn");
    btn.style.setProperty("--p", "0%");
    btn.textContent = t("dl.downloading");
  }
  syncInstallBtns(id);
  try {
    await invoke("install", { programId: id });
  } catch (e) {
    installing.delete(id);
    clearButtonProgress(btn);
    if (btn) btn.textContent = btn.dataset.installed === "1" ? t("dl.update") : t("dl.download");
    syncInstallBtns(id);
    showNotice(String(e), true);
  }
}

function findDlButton(id) {
  if (view === "manage" && current?.id === id) {
    const b = document.querySelector("#dl-btn");
    if (b) return b;
  }
  return document.querySelector(`button[data-program-id="${id}"]`);
}

function setButtonProgress(btn, pct, label) {
  if (!btn) return;
  btn.classList.add("dl-progress-btn");
  btn.style.setProperty("--p", pct + "%");
  btn.textContent = label;
}

function clearButtonProgress(btn) {
  if (!btn) return;
  btn.classList.remove("dl-progress-btn");
  btn.style.removeProperty("--p");
}

function syncInstallBtns(id) {
  const dl = document.querySelector("#dl-btn");
  if (!dl || current?.id !== id || view !== "manage") return;
  const st = statuses.find((s) => s.id === id)?.status;
  const busy = installing.has(id);
  const isLocal = current && !current.repo;
  if (isLocal) {
    dl.hidden = true;
    return;
  }
  if (st?.installed && st?.up_to_date) {
    dl.hidden = true;
    return;
  }
  if (st?.installed && !st?.latest_version) {
    dl.hidden = true;
    return;
  }
  dl.hidden = false;
  dl.dataset.programId = id;
  dl.dataset.installed = st?.installed ? "1" : "0";
  dl.disabled = busy;
  dl.textContent = busy ? t("dl.downloading") : st?.installed ? t("dl.update") : t("dl.download");
}

function handleInstallProgress(p) {
  const { programId, stage, received, total, done, error, version } = p || {};
  const btn = findDlButton(programId);
  if (stage === "downloading") {
    const pct = total ? Math.min(99, Math.round((received / total) * 100)) : null;
    setButtonProgress(btn, pct == null ? 0 : pct, pct == null ? t("dl.downloading") : t("dl.progress", { pct }));
  } else if (stage === "verifying" || stage === "extracting") {
    setButtonProgress(btn, 100, t("dl.working"));
  } else if (done) {
    completeInstall(programId, null, version || "");
  } else if (error) {
    completeInstall(programId, error, "");
  }
}

async function completeInstall(id, error, version) {
  installing.delete(id);
  try {
    if (current?.id === id) await refreshStatusLocal();
    else await refreshAllStatuses();
  } catch {}
  syncInstallBtns(id);
  const btn = findDlButton(id);
  clearButtonProgress(btn);
  if (btn) {
    btn.disabled = false;
    if (error) btn.textContent = btn.dataset.installed === "1" ? t("dl.update") : t("dl.download");
    else { btn.textContent = t("st.latest"); btn.disabled = true; }
  }
  if (error) showNotice(String(error), true);
  else showNotice(t("toast.updated", { name: (programs.find((x) => x.id === id) || {}).name || id, ver: version }));
}

// WS 事件订阅：install 进度 / 将来其它广播
let ws = null;
// 页面隐藏（最小化/切后台）时暂停后台轮询与日志刷新；恢复可见时一次性补刷新。
function pageHidden() {
  return typeof document.hidden !== "undefined" && document.hidden;
}
function connectWS() {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  const q = globalThis.usToken ? "?token=" + encodeURIComponent(globalThis.usToken) : "";
  try {
    ws = new WebSocket(`${proto}//${location.host}/ws${q}`);
  } catch {
    setTimeout(connectWS, 2000);
    return;
  }
  ws.onmessage = (e) => {
    try {
      const m = JSON.parse(e.data);
      if (m && m.type === "install-progress") handleInstallProgress(m);
      else if (m && m.type === "status-change") {
        // F-3/F10：进程状态由服务端推送（无需轮询）；只刷新本地状态，不发网络请求
        refreshAllStatusesCoalesced();
        if (current) refreshManageLogCoalesced();
      } else if (m && m.type === "log-tick") {
        // F-3/F11：日志增量推送提示 → 对当前程序做一次增量 tail 拉取（合并刷新）
        if (current && m.programId === current.id) refreshManageLogCoalesced();
      }
    } catch {}
  };
  ws.onclose = () => setTimeout(connectWS, 2000);
}

// 合并刷新：同一帧内多次事件只触发一次 RPC，避免最小化期间的积压事件风暴
let mlogPending = false;
function refreshManageLogCoalesced() {
  if (pageHidden() || !current || mlogPending) return;
  mlogPending = true;
  requestAnimationFrame(() => {
    mlogPending = false;
    refreshManageLog();
  });
}
let statusPending = false;
function refreshAllStatusesCoalesced() {
  if (pageHidden() || statusPending) return;
  statusPending = true;
  requestAnimationFrame(() => {
    statusPending = false;
    refreshAllStatuses();
  });
}
// 页面重新可见：立即合并刷新一次（最小化期间积压的状态/日志）。
// 后台时 rAF 被挂起、pending 可能残留为 true，这里强制归位并直刷。
document.addEventListener("visibilitychange", () => {
  if (pageHidden()) return;
  mlogPending = false;
  statusPending = false;
  if (current) refreshManageLog();
  refreshAllStatuses();
});

async function checkUpdates(btn) {
  if (checkingUpdates) return;
  checkingUpdates = true;
  const targets = [btn].filter(Boolean);
  targets.forEach((b) => {
    b.classList.add("busy");
    b.disabled = true;
    b.textContent = t("dl.checking_short");
  });
  try {
    const full = await invoke("batch_status");
    statuses = full;
    renderSidebar();
    if (window.usWorkbench) await window.usWorkbench.load();
    if (current) {
      const cur = statuses.find((it) => it.id === current.id)?.status;
      if (cur) applyStatus(cur);
    }
    // 代理/网络不通等场景：有远程源的程序查不到最新版本 → 分类提示失败
    const failed = statuses.filter((it) => hasRemoteSource(it) && !it.status?.latest_version).length;
    if (failed > 0) await netErrorNotice(null, failed);
    else showNotice(t("dl.done"));
  } catch (e) {
    showNotice(String(e), true);
  } finally {
    checkingUpdates = false;
    targets.forEach((b) => {
      b.classList.remove("busy");
      b.disabled = false;
      b.textContent = t("batch.check");
    });
    if (current) {
      const cur = statuses.find((it) => it.id === current.id)?.status;
      if (cur) updateUpdateCard(cur);
    }
  }
}

function hasRemoteSource(item) {
  return !!(item.repo || statusSource(item.id));
}

function statusSource(id) {
  const p = programs.find((x) => x.id === id);
  return p && (!!p.http_enabled);
}

function timeAgo(secs) {
  if (!secs) return t("st.never");
  const diff = Date.now() / 1000 - secs;
  if (diff < 60) return t("st.just");
  if (diff < 3600) return t("time.min_ago", { n: Math.floor(diff / 60) });
  if (diff < 86400) return t("time.hour_ago", { n: Math.floor(diff / 3600) });
  if (diff < 86400 * 30) return t("time.day_ago", { n: Math.floor(diff / 86400) });
  return fmtDate(secs);
}

// ---------- 模板库 ----------
async function ensureLibraryFromCache() {
  try {
    if (manifest) { renderLibrary(); return; }
    manifest = await invoke("get_merged_manifest_offline");
    libPage = 0;
  } catch {}
  renderLibrary();
}

async function refreshLibrary() {
  try {
    const m = await invoke("get_merged_manifest", { registryUrl: registryUrl || "" });
    manifest = m;
    if (libSource && !(manifest.sources || []).some((s) => s[0] === libSource)) libSource = null;
    libPage = 0;
    clearTemplateStatusCache();
    renderLibrary();
    const nOffline = (manifest.sources || []).filter(([, off]) => off).length;
    const msg =
      (manifest.sources || []).length === 0
        ? t("lib.no_sources")
        : t("lib.summary", { n: manifest.sources.length, offline: nOffline ? t("lib.summary_offline", { n: nOffline }) : "", m: manifest.templates.length });
    showNotice(msg, nOffline > 0);
  } catch (e) {
    const msg = t("toast.manifest_fail", { err: e });
    el.libStatus.textContent = msg;
    await netErrorNotice(e);
  }
}

async function refreshTemplateStatus(base, id, btn) {
  const stKey = base + "\u0000" + id;
  try {
    const st = await invoke("template_status", { registryUrl: base || registryUrl || "", templateId: id });
    templateStatusCache[stKey] = st;
    if (st === "current") {
      btn.disabled = true;
      btn.textContent = t("act.latest");
      btn.title = t("act.latest_hint");
    } else if (st === "new") {
      btn.textContent = t("act.import");
      btn.title = "";
    } else {
      btn.textContent = t("act.update");
      btn.title = t("act.update_hint");
    }
  } catch (_) {
    templateStatusCache[stKey] = "update";
  }
}

function renderLibrary() {
  if (!manifest) {
    el.libList.innerHTML = t("lib.empty_remote");
    renderSourceBar();
    renderLocalTemplates();
    return;
  }
  renderSourceBar();
  const nOffline = (manifest.sources || []).filter(([, off]) => off).length;
  el.libStatus.textContent =
    (manifest.sources || []).length === 0
      ? t("lib.no_sources")
      : t("lib.summary", { n: manifest.sources.length, offline: nOffline ? t("lib.summary_offline", { n: nOffline }) : "", m: manifest.templates.length });

  const kw = libSearchValue.trim().toLowerCase();
  const rows = manifest.templates.filter(([id, tpl, base]) => {
    if (libSource && base !== libSource) return false;
    if (!kw) return true;
    const hay = (tpl ? [id, tpl.id, tpl.name, tpl.category, tpl.description].join(" ") : "").toLowerCase();
    return hay.includes(kw);
  });

  // 模板库走「分类分组卡片」（方案 A）：模板是「浏览 → 识别 → 导入一次」，
  // 不需要表格的列对齐与批量操作；分组让同类模板聚在一起，折叠状态只在内存里。
  const groups = new Map();
  for (const row of rows) {
    const cat = (row[1] && row[1].category) || t("lib.uncategorized");
    if (!groups.has(cat)) groups.set(cat, []);
    groups.get(cat).push(row);
  }

  el.libList.innerHTML = "";
  el.libList.classList.add("lib-groups");
  if (!rows.length) {
    el.libList.innerHTML = '<div class="lib-desc">' + t("lib.no_match") + "</div>";
  }
  for (const [cat, list] of groups) {
    const sec = document.createElement("section");
    sec.className = "lib-group";
    const head = document.createElement("button");
    head.type = "button";
    head.className = "lib-group-head";
    const closed = libCollapsed.has(cat);
    head.innerHTML =
      `<span class="lib-group-chev">${closed ? "▶" : "▼"}</span>` +
      `<b>${escHtml(cat)}</b>` +
      `<span class="lib-group-count">${list.length}</span>`;
    head.onclick = () => {
      if (libCollapsed.has(cat)) libCollapsed.delete(cat);
      else libCollapsed.add(cat);
      renderLibrary();
    };
    sec.appendChild(head);
    if (!closed) {
      const grid = document.createElement("div");
      grid.className = "lib-grid";
      for (const [id, tpl, base] of list) grid.appendChild(libCard(id, tpl, base));
      sec.appendChild(grid);
    }
    el.libList.appendChild(sec);
  }
  renderPager(1);
  renderLocalTemplates();
}

// 单个模板卡片：保持原有导入/更新按钮的全部行为，只是搬进分组栅格。
function libCard(id, tpl, base) {
  const card = document.createElement("div");
  card.className = "lib-card";
  const top = document.createElement("div");
  top.className = "lib-top";
  const h = document.createElement("span");
  h.className = "lib-name";
  h.textContent = tpl.name;
  const cat = document.createElement("span");
  cat.className = "lib-cat";
  cat.textContent = `[${tpl.category}]`;
  const repo = document.createElement("span");
  repo.className = "lib-repo";
  repo.textContent = tpl.repo || t("lib.local_program");

  const imported = programs.some((p) => p.id === id);
  const conflict = (manifest.conflicts || []).find(([cid]) => cid === id);
  if (conflict && conflict[1] > 1) {
    const mark = document.createElement("span");
    mark.className = "lib-cat lib-conflict";
    mark.textContent = t("lib.multi_source", { n: conflict[1] });
    top.append(mark);
  }

  const btn = document.createElement("button");
  const stKey = base + "\u0000" + id;
  const status = templateStatusCache[stKey];
  const latest = imported && status === "current";
  btn.textContent = importing.has(id)
    ? t("lib.importing")
    : latest ? t("act.latest") : imported ? t("act.update") : t("act.import");
  btn.disabled = importing.has(id) || latest;
  btn.onclick = () => doImport(id, base, btn);
  if (imported && latest) btn.title = t("act.latest_hint");
  else if (imported) btn.title = t("act.update_hint");
  top.append(h, cat, repo, btn);
  if (imported && status === undefined) refreshTemplateStatus(base, id, btn);

  const desc = document.createElement("div");
  desc.className = "lib-desc";
  desc.textContent = tpl.description;
  const meta = document.createElement("div");
  meta.className = "lib-meta";
  if (imported) {
    const local = document.createElement("span");
    local.className = "lib-local-badge";
    local.textContent = t("lib.local_last");
    local.title = t("lib.imported");
    meta.appendChild(local);
  }
  card.append(top, meta, desc);
  return card;
}

function renderSourceBar() {
  if (!el.libSourceBar) return;
  el.libSourceBar.innerHTML = "";
  const sel = document.createElement("select");
  const allOpt = document.createElement("option");
  allOpt.value = "__all__";
  allOpt.textContent = t("lib.all_sources");
  if (libSource === null) allOpt.selected = true;
  sel.appendChild(allOpt);
  const srcs = manifest && manifest.sources && manifest.sources.length ? manifest.sources : registries.map((r) => [r, false, null]);
  for (const src of srcs) {
    const [base, offline, fetched] = src;
    const opt = document.createElement("option");
    opt.value = base;
    opt.textContent = (offline ? t("lib.offline") : "") + base + (fetched ? ` · ${fmtDate(fetched)}` : "");
    if (libSource === base) opt.selected = true;
    sel.appendChild(opt);
  }
  sel.onchange = () => {
    libSource = sel.value === "__all__" ? null : sel.value;
    libPage = 0;
    renderLibrary();
  };
  el.libSourceBar.appendChild(sel);
  const refresh = document.createElement("button");
  refresh.textContent = t("act.refresh");
  refresh.onclick = async () => {
    refresh.disabled = true;
    refresh.textContent = t("lib.pull");
    try { await refreshLibrary(); } finally {
      refresh.disabled = false;
      refresh.textContent = t("act.refresh");
    }
  };
  el.libSourceBar.appendChild(refresh);
}

function renderPager(pages) {
  if (!el.libPager) return;
  el.libPager.innerHTML = "";
  if (pages <= 1) return;
  const prev = document.createElement("button");
  prev.textContent = t("lib.prev");
  prev.disabled = libPage <= 0;
  prev.onclick = () => { libPage--; renderLibrary(); };
  const info = document.createElement("span");
  info.className = "lib-page-info";
  info.textContent = `${libPage + 1} / ${pages}`;
  const next = document.createElement("button");
  next.textContent = t("lib.next");
  next.disabled = libPage >= pages - 1;
  next.onclick = () => { libPage++; renderLibrary(); };
  el.libPager.append(prev, info, next);
}

function renderLocalTemplates() {
  const list = programs || [];
  if (el.libCacheToggle) el.libCacheToggle.textContent = t("lib.local") + `(${list.length})`;
  if (!el.libCacheDrawer) return;
  el.libCacheDrawer.innerHTML = "";
  const head = document.createElement("h3");
  head.className = "lib-cache-heading";
  head.textContent = t("lib.local_has");
  el.libCacheDrawer.appendChild(head);
  if (!list.length) {
    const d = document.createElement("div");
    d.className = "lib-desc";
    d.textContent = t("lib.empty_local");
    el.libCacheDrawer.appendChild(d);
    return;
  }
  for (const p of list) {
    const row = document.createElement("div");
    row.className = "lib-cache-row";
    const name = document.createElement("span");
    name.className = "lib-name";
    name.textContent = p.name || p.id;
    const repo = document.createElement("span");
    repo.className = "lib-repo";
    repo.textContent = p.repo || "";
    const ops = document.createElement("span");
    ops.className = "lib-ops";
    const exp = document.createElement("button");
    exp.className = "op-btn";
    exp.textContent = t("act.export");
    exp.onclick = () => exportLocalTemplate(p.id, p.name || p.id);
    ops.append(exp);
    row.append(name, repo, ops);
    el.libCacheDrawer.appendChild(row);
  }
}

async function exportLocalTemplate(id, name) {
  try {
    const text = await invoke("export_template_json", { programId: id });
    const blob = new Blob([text], { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${name}.json`;
    a.click();
    URL.revokeObjectURL(a.href);
    showNotice(t("toast.exported", { name }));
  } catch (e) {
    showNotice(String(e), true);
  }
}

async function doImport(id, base, btn) {
  if (importing.has(id)) return;
  if (programs.some((p) => p.id === id)) {
    openTemplateDiffModal(id, base);
    return;
  }
  importing.add(id);
  if (btn) {
    btn.disabled = true;
    btn.textContent = t("lib.importing");
  }
  try {
    await invoke("import_template", { registryUrl: base || registryUrl || "", templateId: id, overwrite: false });
    showNotice(t("toast.import_done", { id }));
    await afterImport(id);
    clearTemplateStatusCache();
  } catch (e) {
    showNotice(String(e), true);
  } finally {
    importing.delete(id);
    if (btn) btn.disabled = false;
    renderLibrary();
  }
}

async function afterImport(id) {
  // 导入后必须走 reloadPrograms：它会同时刷新 programs/statuses 与工作台列表
  // （st.items 是另一份缓存）。只 renderSidebar() 的话，侧栏重画的是旧 items，
  // 新导入的程序要等 5 秒轮询才出现。
  await reloadPrograms();
  if (!current || !programs.some((p) => p.id === current.id)) {
    // 走工作台的 select：它会顺手把侧栏高亮切到新程序（内部再调 switchCurrent
    // 渲染详情）。只 switchCurrent 的话详情换了、侧栏一个都没选中。
    if (window.usWorkbench && typeof window.usWorkbench.select === "function") {
      await window.usWorkbench.select(id);
    } else {
      await switchCurrent(id);
    }
  }
}

let pendingTemplateDiff = null;
const diffModal = document.querySelector("#template-diff-modal");

async function openTemplateDiffModal(id, base) {
  const content = document.querySelector("#template-diff-content");
  const applyBtn = document.querySelector("#template-diff-apply");
  pendingTemplateDiff = { id, base };
  content.textContent = "";
  content.append(t("tmpl_diff.loading"));
  diffModal.hidden = false;
  applyBtn.disabled = false;
  try {
    const d = await invoke("template_diff", { registryUrl: base || registryUrl || "", templateId: id });
    renderTemplateDiff(content, d);
  } catch (e) {
    content.textContent = "";
    content.append(String(e));
  }
}

function renderTemplateDiff(content, d) {
  content.textContent = "";
  const summary = document.createElement("div");
  summary.className = "tmpl-diff-summary";
  summary.textContent = d.summary;
  content.appendChild(summary);
  if (!d.lines || !d.lines.some((l) => l.kind === "+" || l.kind === "-")) {
    const note = document.createElement("div");
    note.className = "tmpl-diff-note";
    note.textContent = t("tmpl_diff.no_diff_detail");
    content.appendChild(note);
    return;
  }
  const pre = document.createElement("div");
  pre.className = "tmpl-diff-code";
  const lineClass = (kind) => kind === "+" ? "tmpl-diff-add" : kind === "-" ? "tmpl-diff-del" : "tmpl-diff-keep";
  const makeRow = (l) => {
    const row = document.createElement("div");
    row.className = lineClass(l.kind);
    row.textContent = (l.kind === " " ? "  " : l.kind + " ") + l.text;
    return row;
  };
  let ctxRun = [];
  for (const l of d.lines) {
    if (l.kind === " ") { ctxRun.push(l); continue; }
    if (ctxRun.length) {
      const details = document.createElement("details");
      const s = document.createElement("summary");
      s.className = "tmpl-diff-summary";
      s.textContent = t("tmpl_diff.expand_context", { n: ctxRun.length });
      details.appendChild(s);
      for (const cl of ctxRun) details.appendChild(makeRow(cl));
      pre.appendChild(details);
      ctxRun = [];
    }
    pre.appendChild(makeRow(l));
  }
  if (ctxRun.length) {
    const details = document.createElement("details");
    const s = document.createElement("summary");
    s.className = "tmpl-diff-summary";
    s.textContent = t("tmpl_diff.expand_context", { n: ctxRun.length });
    details.appendChild(s);
    for (const cl of ctxRun) details.appendChild(makeRow(cl));
    pre.appendChild(details);
  }
  content.appendChild(pre);
}

function closeTemplateDiffModal() {
  diffModal.hidden = true;
  pendingTemplateDiff = null;
}

async function applyTemplateUpdate() {
  if (!pendingTemplateDiff) return;
  const { id, base } = pendingTemplateDiff;
  closeTemplateDiffModal();
  if (importing.has(id)) return;
  importing.add(id);
  try {
    await invoke("import_template", { registryUrl: base || registryUrl || "", templateId: id, overwrite: true });
    showNotice(t("toast.import_overwritten", { id }));
    await afterImport(id);
    clearTemplateStatusCache();
  } catch (e) {
    showNotice(String(e), true);
  } finally {
    importing.delete(id);
    renderLibrary();
  }
}

function clearTemplateStatusCache() {
  for (const k of Object.keys(templateStatusCache)) delete templateStatusCache[k];
}

// ---------- 模板源管理 ----------
function openSourcesModal() {
  renderSourcesList();
  document.querySelector("#sources-modal").hidden = false;
}

function sourceRows() {
  return [...document.querySelectorAll("#sources-list .source-row")];
}

function renderSourcesList() {
  const box = document.querySelector("#sources-list");
  box.innerHTML = "";
  (registries.length ? registries : [""]).forEach((r, i) => box.appendChild(sourceRowEl(r, i === 0)));
}

function sourceRowEl(url, isDefault) {
  const row = document.createElement("div");
  row.className = "source-row" + (isDefault ? " default" : "");
  const input = document.createElement("input");
  input.value = url;
  input.placeholder = t("sources.placeholder");
  const tag = document.createElement("span");
  tag.className = "lib-cat";
  tag.textContent = isDefault ? t("lib.default_rule") : "";
  row.append(input, tag);
  if (!isDefault) {
    const del = document.createElement("button");
    del.className = "edit-field-del";
    del.type = "button";
    del.textContent = "✕";
    del.title = t("lib.delete_source");
    del.onclick = () => row.remove();
    row.append(del);
  }
  return row;
}

document.querySelector("#sources-add-btn").onclick = () => {
  const val = document.querySelector("#sources-new").value.trim();
  document.querySelector("#sources-list").appendChild(sourceRowEl(val, false));
  document.querySelector("#sources-new").value = "";
};

async function saveSources() {
  const list = sourceRows().map((r) => r.querySelector("input").value.trim()).filter(Boolean);
  if (!list.length) { showNotice(t("lib.keep_one"), true); return; }
  try {
    await invoke("set_registries", { registries: list });
    registries = await invoke("get_registries");
    registryUrl = registries[0] ?? "";
    libSource = registries[0] ?? null;
    showNotice(t("toast.sources_saved"));
    document.querySelector("#sources-modal").hidden = true;
    manifest = null;
    renderLibrary();
  } catch (e) {
    showNotice(String(e), true);
  }
}

// ---------- 网络 / 代理设置 ----------
function parseProxy(url) {
  const out = { type: "http", host: "", user: "", pass: "" };
  if (!url) return out;
  let s = String(url).trim();
  const m = s.match(/^([a-z][a-z0-9+.-]*):\/\//i);
  if (m) {
    out.type = m[1].toLowerCase();
    s = s.slice(m[0].length);
  }
  const at = s.lastIndexOf("@");
  if (at >= 0) {
    const cred = s.slice(0, at);
    s = s.slice(at + 1);
    const ci = cred.indexOf(":");
    if (ci >= 0) {
      out.user = decodeURIComponent(cred.slice(0, ci));
      out.pass = decodeURIComponent(cred.slice(ci + 1));
    } else out.user = decodeURIComponent(cred);
  }
  out.host = s;
  if (out.type === "socks5h" || out.type === "socks") out.type = "socks5";
  if (out.type === "https") out.type = "http";
  return out;
}

function buildProxy(type, host, user, pass) {
  if (!host) return "";
  const cred = user || pass ? `${encodeURIComponent(user)}:${encodeURIComponent(pass)}@` : "";
  return `${type}://${cred}${host}`;
}

function escHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

const settingsModal = document.querySelector("#settings-modal");
// 设置面板缓存的网络配置（格式与 web-server rpc 一致）
let settAccel = []; // {id,url}
let settProxies = []; // {id,url}
let settSelectedAccel = "";
let settSelectedProxy = "";
let settAccelPings = {}; // url -> ms | null
let settProxyPings = {}; // url -> ms | null
let settNetEditId = "";   // 正在编辑的 id（在哪个数组里由 kind 决定）；空=新增

function openSettings() {
  const typeEl = document.querySelector("#sett-net-type");
  const hostEl = document.querySelector("#sett-net-host");
  const userEl = document.querySelector("#sett-net-user");
  const passEl = document.querySelector("#sett-net-pass");
  const onEl = document.querySelector("#sett-proxy-enabled");
  typeEl.value = "acc";
  hostEl.value = "";
  userEl.value = "";
  passEl.value = "";
  settNetEditId = "";
  updateNetAddline();
  invoke("get_proxy")
    .then((p) => {
      onEl.checked = p.proxy_enabled !== false;
      settAccel = (p.accelerate_presets || []).map((e) => ({ id: e.id, url: e.url }));
      settProxies = (p.saved_proxies || []).map((e) => ({ id: e.id, url: e.url }));
      settSelectedAccel = p.selected_accelerate || "";
      settSelectedProxy = p.selected_proxy || "";
      // 单选列表：代理优先于加速，若同时选中则只保留代理
      if (settSelectedProxy) settSelectedAccel = "";
      renderSettNet();
    })
    .catch((e) => showNotice(String(e), true));
  invoke("shell_autostart_enabled")
    .then((on) => { document.querySelector("#sett-shell-auto").checked = !!on; })
    .catch(() => {});
  invoke("get_web_settings")
    .then((w) => {
      document.querySelector("#sett-web-port").value = w.port || 0;
      document.querySelector("#sett-lan").checked = w.bind !== "";
      document.querySelector("#sett-token").value = w.token || "";
    })
    .catch(() => {});
  invoke("get_shell_version")
    .then((v) => {
      if (!shellUpdate) shellUpdate = { current: v, latest_tag: null, release_url: null };
      renderShellUpdate();
    })
    .catch(() => {});
  settingsModal.hidden = false;
}

function settNetItems() {
  return [
    ...settAccel.map((e) => ({ ...e, kind: "acc" })),
    ...settProxies.map((e) => ({ ...e, kind: "proxy" })),
  ];
}

function settEffItem() {
  // 代理优先于加速地址
  const p = settSelectedProxy ? settProxies.find((e) => e.id === settSelectedProxy) : null;
  if (p) return { ...p, kind: "proxy" };
  const a = settSelectedAccel ? settAccel.find((e) => e.id === settSelectedAccel) : null;
  if (a) return { ...a, kind: "acc" };
  return null;
}

function renderSettNet() {
  const eff = settEffItem();
  const onEl = document.querySelector("#sett-proxy-enabled");
  const effEl = document.querySelector("#sett-net-effective");
  const enabled = onEl && onEl.checked;
  if (!enabled) {
    effEl.innerHTML = `<span class="eff-ico">○</span><span>${t("sett.eff_off")}</span>`;
  } else if (!eff) {
    effEl.innerHTML = `<span class="eff-ico">○</span><span>${t("sett.eff_none")}</span>`;
  } else {
    const ms = (eff.kind === "proxy" ? settProxyPings : settAccelPings)[eff.url];
    const kindLabel = eff.kind === "proxy" ? t("sett.kind_proxy") : t("sett.kind_acc");
    const ping = ms === undefined ? ""
      : ms === null ? ` · ${t("sett.ping_fail")}`
      : ` · ${ms} ms`;
    effEl.innerHTML = `<span class="eff-ico">${eff.kind === "proxy" ? "🚀" : "⚡"}</span>` +
      `<span class="eff-kind">${kindLabel}</span>` +
      `<code class="eff-url" title="${escHtml(eff.url)}">${escHtml(eff.url)}</code>` +
      `<span class="eff-ping">${ping}</span>`;
  }
  document.querySelector(".sett-proxy").classList.toggle("off", !enabled);

  const items = settNetItems();
  const box = document.querySelector("#sett-net-list");
  if (!box) return;
  if (!items.length) {
    box.innerHTML = `<div class="sett-proxy-empty">${t("sett.empty")}</div>`;
    return;
  }
  box.innerHTML = items
    .map((e) => {
      const pings = e.kind === "proxy" ? settProxyPings : settAccelPings;
      const ms = pings[e.url];
      const pingHtml = ms === undefined
        ? `<span class="ping measuring" data-url="${escHtml(e.url)}">…</span>`
        : ms === null
          ? `<span class="ping fail" data-url="${escHtml(e.url)}">${t("sett.ping_fail")}</span>`
          : `<span class="ping" data-url="${escHtml(e.url)}">${ms} ms</span>`;
      const kindLabel = e.kind === "proxy" ? t("sett.kind_proxy") : t("sett.kind_acc");
      const icon = e.kind === "proxy" ? "🚀" : "⚡";
      const selId = e.kind === "proxy" ? settSelectedProxy : settSelectedAccel;
      return `<div class="sett-item ${e.id === selId ? "selected" : ""}" data-id="${escHtml(e.id)}">
        <input type="radio" name="sett-net-choice" ${e.id === selId ? "checked" : ""} data-id="${escHtml(e.id)}" />
        <span class="kind-tag kind-${e.kind}" title="${kindLabel}">${icon} ${kindLabel}</span>
        <span class="url" title="${escHtml(e.url)}">${escHtml(e.url)}</span>
        ${pingHtml}
        <button type="button" class="op-edit" data-edit="${escHtml(e.id)}" title="${t("sett.edit")}" data-i18n-aria="sett.edit">✎</button>
        <button type="button" class="op-danger" data-del="${escHtml(e.id)}" title="${t("sett.remove")}" data-i18n-aria="sett.remove">×</button>
      </div>`;
    })
    .join("");
  box.querySelectorAll("input[type=radio]").forEach((r) =>
    r.addEventListener("change", () => {
      const it = settNetItems().find((e) => e.id === r.dataset.id);
      if (!it) return;
      if (it.kind === "proxy") { settSelectedProxy = it.id; settSelectedAccel = ""; }
      else { settSelectedAccel = it.id; settSelectedProxy = ""; }
      renderSettNet();
    }));
  box.querySelectorAll("[data-del]").forEach((b) =>
    b.addEventListener("click", () => {
      const it = settNetItems().find((e) => e.id === b.dataset.del);
      if (!it) return;
      if (it.kind === "proxy") {
        settProxies = settProxies.filter((e) => e.id !== it.id);
        settProxyPings = {};
        if (settSelectedProxy === it.id) settSelectedProxy = "";
      } else {
        settAccel = settAccel.filter((e) => e.id !== it.id);
        settAccelPings = {};
        if (settSelectedAccel === it.id) settSelectedAccel = "";
      }
      if (settNetEditId === it.id) clearNetEdit();
      renderSettNet();
    }));
  box.querySelectorAll("[data-edit]").forEach((b) =>
    b.addEventListener("click", () => startNetEdit(b.dataset.edit)));
}

function clearNetEdit() {
  settNetEditId = "";
  const btn = document.querySelector("#sett-net-submit");
  if (btn) btn.textContent = t("act.add");
  updateNetAddline();
}

function updateNetAddline() {
  const type = document.querySelector("#sett-net-type").value;
  const cred = document.querySelector("#sett-net-cred");
  const host = document.querySelector("#sett-net-host");
  if (cred) cred.hidden = type === "acc";
  if (host) host.placeholder = type === "acc"
    ? t("sett.acc_placeholder")
    : t("sett.proxy_placeholder");
}

function startNetEdit(id) {
  const it = settNetItems().find((e) => e.id === id);
  if (!it) return;
  settNetEditId = id;
  const typeEl = document.querySelector("#sett-net-type");
  const hostEl = document.querySelector("#sett-net-host");
  const userEl = document.querySelector("#sett-net-user");
  const passEl = document.querySelector("#sett-net-pass");
  const btn = document.querySelector("#sett-net-submit");
  if (it.kind === "acc") {
    typeEl.value = "acc";
    hostEl.value = it.url;
    userEl.value = "";
    passEl.value = "";
  } else {
    const p = parseProxy(it.url);
    typeEl.value = p.type;
    hostEl.value = p.host;
    userEl.value = p.user;
    passEl.value = p.pass;
  }
  updateNetAddline();
  if (btn) btn.textContent = t("act.save");
  renderSettNet();
}

function settNetSubmit() {
  const type = document.querySelector("#sett-net-type").value;
  const host = document.querySelector("#sett-net-host").value.trim();
  const user = document.querySelector("#sett-net-user").value.trim();
  const pass = document.querySelector("#sett-net-pass").value;
  let url;
  if (type === "acc") {
    url = host;
    if (!/^https?:\/\//i.test(url)) { showNotice(t("sett.url_invalid"), true); return; }
  } else {
    url = buildProxy(type, host, user, pass);
    if (!url) { showNotice(t("sett.proxy_placeholder"), true); return; }
  }
  if (settNetEditId) {
    const it = settNetItems().find((e) => e.id === settNetEditId);
    if (it) {
      const list = it.kind === "proxy" ? settProxies : settAccel;
      if (list.some((e) => e.id !== it.id && e.url === url)) { showNotice(t("sett.dup"), true); return; }
      const idx = list.findIndex((e) => e.id === it.id);
      list[idx] = { ...list[idx], url };
      if (it.kind === "proxy") settProxyPings = {};
      else settAccelPings = {};
    }
    clearNetEdit();
  } else {
    if (type === "acc") {
      if (settAccel.some((e) => e.url === url)) { showNotice(t("sett.dup"), true); return; }
      const id = "a" + (+new Date()).toString(36) + Math.random().toString(36).slice(2, 6);
      settAccel.push({ id, url });
      settSelectedAccel = id;
      settSelectedProxy = "";
    } else {
      if (settProxies.some((e) => e.url === url)) { showNotice(t("sett.dup"), true); return; }
      const id = "p" + (+new Date()).toString(36) + Math.random().toString(36).slice(2, 6);
      settProxies.push({ id, url });
      settSelectedProxy = id;
      settSelectedAccel = "";
    }
  }
  document.querySelector("#sett-net-host").value = "";
  document.querySelector("#sett-net-user").value = "";
  document.querySelector("#sett-net-pass").value = "";
  renderSettNet();
}

function measureNetPings() {
  const items = settNetItems();
  if (!items.length) { showNotice(t("sett.ping_none")); return; }
  settAccelPings = {};
  settProxyPings = {};
  renderSettNet();
  showNotice(t("sett.ping_start"));
  let done = 0;
  const total = items.length;
  // 每个目标单独发起，完成后即逐行展示，不等最慢的超时目标
  items.forEach((it) => {
    const kind = it.kind === "proxy" ? "proxy" : "url";
    invoke("check_pings", { targets: [{ url: it.url, kind }] })
      .then((list) => {
        const arr = (list && list.pings) || [];
        const v = arr[0] != null ? arr[0] : null;
        if (it.kind === "proxy") settProxyPings[it.url] = v;
        else settAccelPings[it.url] = v;
        renderSettNet();
      })
      .catch(() => {
        if (it.kind === "proxy") settProxyPings[it.url] = null;
        else settAccelPings[it.url] = null;
        renderSettNet();
      })
      .finally(() => {
        done += 1;
        if (done >= total) {
          const ok = Object.values(settProxyPings).filter((v2) => v2 != null).length
            + Object.values(settAccelPings).filter((v2) => v2 != null).length;
          showNotice(t("sett.ping_done", { ok, total }));
        }
      });
  });
}

async function saveSettings() {
  const proxyEnabled = document.querySelector("#sett-proxy-enabled").checked;
  const shellAuto = document.querySelector("#sett-shell-auto").checked;
  const webPort = Number(document.querySelector("#sett-web-port").value) || 0;
  const webBind = document.querySelector("#sett-lan").checked ? "0.0.0.0" : "";
  const webToken = document.querySelector("#sett-token").value.trim();
  try {
    await invoke("set_proxy", {
      proxyEnabled,
      acceleratePresets: settAccel,
      savedProxies: settProxies,
      selectedAccelerate: settSelectedAccel,
      selectedProxy: settSelectedProxy,
    });
    try {
      await invoke("set_shell_autostart", { enabled: shellAuto });
    } catch (e) {
      showNotice(t("toast.shell_autostart_fail", { err: e }), true);
    }
    try {
      await invoke("set_web_settings", { bind: webBind, port: webPort, token: webToken });
    } catch (e) {
      showNotice(String(e), true);
    }
    showNotice(t("toast.settings_saved"));
  } catch (e) {
    showNotice(String(e), true);
  }
  settingsModal.hidden = true;
}

function renderShellUpdate() {
  const verEl = document.querySelector("#sett-shell-version");
  if (verEl) verEl.textContent = shellUpdate ? t("upd.current_version", { ver: shellUpdate.current }) : "";
  const link = document.querySelector("#sett-update-link");
  const linkText = document.querySelector("#sett-update-text");
  if (link) {
    const has = !!(shellUpdate && shellUpdate.latest_tag);
    link.hidden = !has;
    if (has) {
      linkText.textContent = t("upd.goto_download") + " " + shellUpdate.latest_tag;
      link.href = shellUpdate.release_url;
    }
  }
  const btn = document.querySelector("#sett-check-update");
  if (btn) {
    btn.disabled = shellChecking;
    btn.textContent = shellChecking ? t("upd.checking") : t("upd.check");
  }
}

async function checkShellUpdate(manual) {
  if (shellChecking) return;
  shellChecking = true;
  renderShellUpdate();
  try {
    const r = await invoke("check_shell_update");
    shellUpdate = r;
    if (r.latest_tag) showNotice(t("upd.available", { latest: r.latest_tag, current: r.current }), false, 8000);
    else if (manual) showNotice(t("upd.latest", { ver: r.current }));
  } catch (e) {
    if (manual) showNotice(t("upd.fail", { err: e }), true);
  } finally {
    shellChecking = false;
    renderShellUpdate();
  }
}

function fmtDate(secs) {
  if (!secs) return "—";
  try { return new Date(secs * 1000).toLocaleString(); } catch { return String(secs); }
}

// ---------- 日志 ----------
// 管理页小日志窗按「最新在上」倒序展示：DOM 里第一条就是最新一行。
// 行数上限只保留最新的 N 条，防长时间运行 prepend 无界增长
// （恢复界面时一次渲染数千条 div 是假死主因之一）。
const MANAGE_LOG_MAX_LINES = 800;
// 倒序后最旧的行在末尾，裁剪要从尾部裁，留下顶部最新的 N 行。
function trimContainerLines(container, max) {
  while (container.childElementCount > max) {
    container.lastChild && container.removeChild(container.lastChild);
  }
}

// 拆行：后端按行写入，文本以 "\n" 结尾时 split 会多出一个空元素，
// 直接渲染会在倒序顶部凭空多一条空行，先丢掉它。
function logLines(text) {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  if (lines.length && lines[lines.length - 1] === "") lines.pop();
  return lines;
}

// 一行 → 一个 div；\x1F 前缀表示 stderr（着色用）
function logLineEl(line) {
  const isErr = line.startsWith("\u001f");
  const div = document.createElement("div");
  div.className = isErr ? "log-err" : "";
  div.textContent = isErr ? line.slice(1) : line;
  return div;
}

// F-3 管理页小日志窗：整段重渲时倒序铺开，最新一屏就在顶部
// （倒序后 scrollTop=0 才是「看到最新输出」，滚到底反而是最旧的一行）
function renderLogBody(container, text) {
  container.innerHTML = "";
  const frag = document.createDocumentFragment();
  for (const line of logLines(text).reverse()) frag.appendChild(logLineEl(line));
  container.appendChild(frag);
  trimContainerLines(container, MANAGE_LOG_MAX_LINES);
  container.scrollTop = 0;
}

// F-3/F11：增量日志尾随。按程序记录上次 EOF 偏移；文件被截断时 reset 整体重渲。
const logOffsets = {};
// 管理页日志窗口当前所属程序 id；切换程序时强制全量重渲避免残留旧程序内容
let manageLogOwner = null;
// 新行插到顶部。用户正往下翻着看旧日志时不能把他弹回顶部：按插入前后的
// scrollHeight 差补偿 scrollTop，正在读的那几行视觉位置不动。
function prependLogText(container, delta) {
  const lines = logLines(delta);
  if (!lines.length) return;
  const atTop = container.scrollTop <= 4;
  const beforeHeight = container.scrollHeight;
  const frag = document.createDocumentFragment();
  for (const line of lines.reverse()) frag.appendChild(logLineEl(line));
  container.insertBefore(frag, container.firstChild);
  trimContainerLines(container, MANAGE_LOG_MAX_LINES);
  container.scrollTop = atTop
    ? 0
    : container.scrollTop + (container.scrollHeight - beforeHeight);
}

async function refreshManageLog() {
  if (!current) return;
  const box = el.manageLogContent;
  const switchedProgram = manageLogOwner !== current.id;
  try {
    const hasOffset = !switchedProgram && Object.prototype.hasOwnProperty.call(logOffsets, current.id);
    const args = { programId: current.id };
    if (hasOffset) args.offset = logOffsets[current.id];
    const res = await invoke("get_logs", args);
    logOffsets[current.id] = res.offset;
    if (!hasOffset || res.reset) renderLogBody(box, res.text);
    else if (res.text) prependLogText(box, res.text);
    manageLogOwner = current.id;
  } catch {
    /* 无日志文件或读取失败：保持现状 */
  }
}

// ---------- 日志中心 ----------
let logCenter = { kind: "shell", id: "", text: "", search: "", follow: true };
let logCenterTimer = null;
let logCenterOffsets = {};

function switchLogCenterView() {
  switchView("log");
  const fb = document.querySelector("#log-follow");
  fb.textContent = t("log.follow") + (logCenter.follow ? " ✓" : "");
  renderLogSources();
  loadLogSource(logCenter.kind, logCenter.id, true);
  startLogCenterTailing();
}

function openLogCenter() {
  switchLogCenterView();
}

function stopLogCenterTailing() {
  if (logCenterTimer) {
    clearInterval(logCenterTimer);
    logCenterTimer = null;
  }
}

function startLogCenterTailing() {
  stopLogCenterTailing();
  if (view !== "log" || !logCenter.follow) return;
  logCenterTimer = setInterval(async () => {
    // 页面隐藏时暂停尾随；恢复可见由 visibilitychange 一次性刷新
    if (pageHidden()) return;
    try {
      await loadLogSource(logCenter.kind, logCenter.id, false);
    } catch {}
  }, 2000);
}

function renderLogSources() {
  const box = el.logSources;
  box.innerHTML = "";
  const sel = el.logSourceSelect;
  if (sel) {
    sel.innerHTML = "";
    sel.hidden = false;
  }
  const addOpt = (kind, id, label, running) => {
    if (!sel) return;
    const o = document.createElement("option");
    o.value = kind + "\u0000" + id;
    o.textContent = (running ? "● " : "○ ") + label;
    if (logCenter.kind === kind && logCenter.id === id) o.selected = true;
    sel.appendChild(o);
  };
  const mk = (kind, id, label, running) => {
    const b = document.createElement("button");
    b.className = "log-src" + (logCenter.kind === kind && logCenter.id === id ? " active" : "");
    const dot = document.createElement("span");
    dot.className = "src-dot" + (running ? " on" : "");
    b.appendChild(dot);
    b.appendChild(document.createTextNode(label));
    b.onclick = () => {
      logCenter.kind = kind;
      logCenter.id = id;
      renderLogSources();
      loadLogSource(kind, id, true);
    };
    box.appendChild(b);
  };
  const sources = [
    ["shell", "", t("log.shell_title"), false],
    ...statuses.filter((s) => s.status.installed).map((s) => ["program", s.id, s.name, !!s.status.running]),
  ];
  for (const [kind, id, label, running] of sources) {
    mk(kind, id, label, running);
    addOpt(kind, id, label, running);
  }
  if (sel) {
    sel.onchange = () => {
      const [kind, id] = sel.value.split("\u0000");
      logCenter.kind = kind;
      logCenter.id = id;
      renderLogSources();
      loadLogSource(kind, id, true);
    };
  }
}

async function loadLogSource(kind, id, reset) {
  let text = "";
  if (kind === "shell") {
    text = await invoke("get_shell_log");
    if (reset || !logCenterOffsets.shell) logCenterOffsets.shell = 0;
    logCenter.text = text;
    capTextLines();
  } else {
    try {
      const args = { programId: id };
      if (!reset && logCenterOffsets[id] != null) args.offset = logCenterOffsets[id];
      const res = await invoke("get_logs", args);
      if (reset || res.reset) logCenter.text = res.text;
      else logCenter.text += res.text;
      logCenterOffsets[id] = res.offset;
      // 封顶：日志中心文本只保留末尾 LOG_CENTER_MAX_LINES 行，防长时间运行 DOM 无限膨胀
      capTextLines();
    } catch {
      if (reset) logCenter.text = t("shell_log.empty");
    }
  }
  logCenter.kind = kind;
  logCenter.id = id;
  el.logClearSrcBtn.hidden = kind !== "shell";
  renderLogContent();
}

// 日志中心文本行数上限：仅保留末尾 N 行（含搜索），超限即截断。
const LOG_CENTER_MAX_LINES = 3000;
function capTextLines() {
  let t = logCenter.text || "";
  let idx = 0;
  let count = 0;
  let pos = 0;
  while (count < LOG_CENTER_MAX_LINES && pos < t.length) {
    const nl = t.indexOf("\n", pos);
    if (nl < 0) {
      count++;
      idx = t.length;
      pos = t.length;
    } else {
      count++;
      idx = nl + 1;
      pos = nl + 1;
    }
  }
  if (count >= LOG_CENTER_MAX_LINES) logCenter.text = t.slice(idx);
}

function renderLogContent() {
  const q = logCenter.search.trim().toLowerCase();
  const container = el.logContent;
  const wasAtBottom = container.scrollTop + container.clientHeight >= container.scrollHeight - 80;
  // 阅读中不跟随：整段重建后 DOM 滚动位置会被清零。
  // 记录当前视口顶部所在行的索引，重建后按行号恢复位置（新行继续追加下方），
  // 避免「跟随开启时滚轮上翻，每 2s 轮询被弹回顶部/底部」的跳来跳去。
  let anchorIndex = -1;
  if (!wasAtBottom && container.children.length) {
    const top = container.scrollTop;
    for (let i = 0; i < container.children.length; i++) {
      if (container.children[i].offsetTop + container.children[i].offsetHeight > top) {
        anchorIndex = i;
        break;
      }
    }
  }
  container.innerHTML = "";
  let matched = 0;
  const text = logCenter.text || "";
  const frag = document.createDocumentFragment();
  for (const rawLine of text.replace(/\r\n?/g, "\n").split("\n")) {
    const isErr = rawLine.startsWith("\u001f");
    const line = isErr ? rawLine.slice(1) : rawLine;
    if (q && !line.toLowerCase().includes(q)) continue;
    matched++;
    const div = document.createElement("div");
    div.className = isErr ? "log-err" : "";
    div.textContent = line;
    frag.appendChild(div);
  }
  if (!q && !matched) {
    const empty = document.createElement("div");
    empty.className = "log-muted";
    empty.textContent = t("shell_log.empty");
    frag.appendChild(empty);
  }
  container.appendChild(frag);
  if (logCenter.follow && wasAtBottom) {
    // 跟随模式 + 本来就在底部：固定钉在最新一行
    container.scrollTop = container.scrollHeight;
    return;
  }
  if (anchorIndex >= 0 && container.children[anchorIndex]) {
    container.scrollTop = container.children[anchorIndex].offsetTop;
  }
}

// ---------- 周期轮询：全部程序状态（对齐桌面端最近修复的全局刷新） ----------
async function refreshAllStatuses() {
  let all;
  try {
    all = await invoke("batch_status_local");
  } catch {
    return;
  }
  const changed = all.some((s) => {
    const prev = statuses.find((x) => x.id === s.id);
    return !prev || prev.status?.running !== s.status?.running;
  });
  statuses = all;
  if (changed || !statuses.length) {
    renderSidebar();
    if (isMobile() && mobPage === "prog") renderMobCards();
    if (view === "log") renderLogSources();
    if (current) {
      const st = all.find((s) => s.id === current.id)?.status;
      if (st) applyStatus(st);
    }
  }
}

// ---------- 程序管理：新建/编辑/复制/删除/显隐 ----------
let editing = null; // { isNew, id, ... } 编辑缓冲

function openEditModal(p) {
  editing = p
    ? {
        isNew: false,
        id: p.id,
        name: p.name,
        binary: p.binary,
        repo: p.repo,
        description: p.description,
        args: [...(p.args || [])],
        env: (p.env || []).map((e) => ({ key: e.key, value: e.value, label: e.label })),
        fields: (p.fields || []).map((f) => ({
          key: f.key,
          kind: f.kind,
          label: f.label,
          default: f.default,
          placeholder: f.placeholder || "",
          required: !!f.required,
        })),
        http_enabled: !!p.http_enabled,
        http_version_url: p.http_version_url || "",
        http_version_json_path: p.http_version_json_path || "",
        http_version_regex: p.http_version_regex || "",
        http_sha256_url: p.http_sha256_url || "",
        http_urls: [...(p.http_urls || [])],
      }
    : {
        isNew: true,
        id: "",
        name: "",
        binary: "",
        repo: "",
        description: "",
        args: [],
        env: [],
        fields: [],
        http_enabled: false,
        http_version_url: "",
        http_version_json_path: "",
        http_version_regex: "",
        http_sha256_url: "",
        http_urls: [],
      };
  document.querySelector("#edit-modal-title").textContent = p
    ? t("edit.title")
    : t("menu.new");
  document.querySelector("#edit-id").disabled = !editing.isNew;
  document.querySelector("#edit-id").value = editing.id;
  document.querySelector("#edit-name").value = editing.name;
  document.querySelector("#edit-binary").value = editing.binary;
  document.querySelector("#edit-repo").value = editing.repo;
  document.querySelector("#edit-desc").value = editing.description;
  document.querySelector("#edit-args").value = editing.args.join("\n");
  document.querySelector("#edit-http-enabled").checked = editing.http_enabled;
  document.querySelector("#edit-http-version-url").value = editing.http_version_url;
  document.querySelector("#edit-http-json-path").value = editing.http_version_json_path;
  document.querySelector("#edit-http-regex").value = editing.http_version_regex;
  document.querySelector("#edit-http-sha256").value = editing.http_sha256_url;
  document.querySelector("#edit-http-urls").value = editing.http_urls.join("\n");
  renderEditHttp();
  renderEnvRows();
  renderFieldRows();
  document.querySelector("#edit-modal").hidden = false;
}

function renderEditHttp() {
  document.querySelector("#edit-http-body").hidden = !document.querySelector("#edit-http-enabled").checked;
}
document.querySelector("#edit-http-enabled").addEventListener("change", renderEditHttp);

function renderEnvRows() {
  const body = document.querySelector("#edit-env-body");
  body.innerHTML = "";
  editing.env.forEach((e, i) => {
    const row = document.createElement("div");
    row.className = "edit-row env";
    const k = document.createElement("input");
    k.placeholder = "KEY";
    k.value = e.key;
    k.oninput = () => (editing.env[i].key = k.value);
    const v = document.createElement("input");
    v.placeholder = t("edit.env_value_ph");
    v.value = e.value;
    v.oninput = () => (editing.env[i].value = v.value);
    const l = document.createElement("input");
    l.placeholder = t("edit.env_label_ph");
    l.value = e.label;
    l.oninput = () => (editing.env[i].label = l.value);
    const del = document.createElement("button");
    del.className = "log-action";
    del.textContent = "×";
    del.onclick = () => {
      editing.env.splice(i, 1);
      renderEnvRows();
    };
    row.append(k, v, l, del);
    body.appendChild(row);
  });
}

function renderFieldRows() {
  const body = document.querySelector("#edit-field-body");
  body.innerHTML = "";
  editing.fields.forEach((f, i) => {
    const row = document.createElement("div");
    row.className = "edit-row";
    const k = document.createElement("input");
    k.placeholder = t("lib.field_key");
    k.value = f.key;
    k.oninput = () => (editing.fields[i].key = k.value);
    const lab = document.createElement("input");
    lab.placeholder = t("edit.field_label");
    lab.value = f.label;
    lab.oninput = () => (editing.fields[i].label = lab.value);
    const kind = document.createElement("select");
    for (const kd of ["string", "boolean", "file", "directory"]) {
      const o = document.createElement("option");
      o.value = kd;
      o.textContent = kd;
      kind.appendChild(o);
    }
    kind.value = f.kind;
    kind.onchange = () => {
      editing.fields[i].kind = kind.value;
      renderFieldRows();
    };
    const def = document.createElement("input");
    def.placeholder = t("lib.def_val");
    def.value = f.default;
    def.oninput = () => (editing.fields[i].default = def.value);
    const ph = document.createElement("input");
    ph.placeholder = t("edit.field_placeholder_ph");
    ph.value = f.placeholder;
    ph.oninput = () => (editing.fields[i].placeholder = ph.value);
    ph.title = t("edit.field_placeholder_ph");
    const req = document.createElement("span");
    req.className = "req";
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = f.required;
    cb.title = t("lib.required");
    cb.onchange = () => (editing.fields[i].required = cb.checked);
    req.appendChild(cb);
    const del = document.createElement("button");
    del.className = "log-action";
    del.textContent = "×";
    del.onclick = () => {
      editing.fields.splice(i, 1);
      renderFieldRows();
    };
    // 文件/目录类型：默认值旁提供「浏览」按钮
    let browse = null;
    if ((f.kind === "file" || f.kind === "directory") && globalThis.haveNativePick) {
      browse = document.createElement("button");
      browse.type = "button";
      browse.className = "op-btn";
      browse.textContent = t("act.browse");
      browse.onclick = async () => {
        try {
          const r = await invoke(f.kind === "file" ? "pick_file" : "pick_directory");
          if (r.supported && r.path) {
            editing.fields[i].default = r.path;
            def.value = r.path;
          }
        } catch (e) {
          showNotice(String(e), true);
        }
      };
    }
    const wrap = document.createElement("div");
    wrap.className = "edit-row-fields";
    row.append(k, lab, kind, wrap);
    wrap.append(def, ph, req);
    if (browse) wrap.appendChild(browse);
    wrap.appendChild(del);
    body.appendChild(row);
  });
}

async function saveEdit() {
  const payload = {
    id: document.querySelector("#edit-id").value.trim(),
    name: document.querySelector("#edit-name").value.trim(),
    binary: document.querySelector("#edit-binary").value.trim(),
    repo: document.querySelector("#edit-repo").value.trim(),
    description: document.querySelector("#edit-desc").value.trim(),
    args: document
      .querySelector("#edit-args")
      .value.split("\n")
      .map((a) => a.trim())
      .filter(Boolean),
    env: editing.env.filter((e) => e.key.trim()),
    fields: editing.fields.filter((f) => f.key.trim()),
    http_enabled: document.querySelector("#edit-http-enabled").checked,
    http_version_url: document.querySelector("#edit-http-version-url").value.trim(),
    http_version_json_path: document.querySelector("#edit-http-json-path").value.trim(),
    http_version_regex: document.querySelector("#edit-http-regex").value.trim(),
    http_sha256_url: document.querySelector("#edit-http-sha256").value.trim(),
    http_urls: document
      .querySelector("#edit-http-urls")
      .value.split("\n")
      .map((a) => a.trim())
      .filter(Boolean),
  };
  if (!payload.id) {
    showNotice(t("err.empty_id"), true);
    return;
  }
  try {
    await invoke(editing.isNew ? "add_program" : "edit_program", { payload });
    document.querySelector("#edit-modal").hidden = true;
    showNotice(t("toast.saved"));
    await reloadPrograms();
    switchCurrent(payload.id);
  } catch (e) {
    showNotice(String(e), true);
  }
}

// 「程序列表变了」的唯一刷新入口：新增 / 删除 / 复制 / 导入 / 改名都走这里。
//
// 注意侧栏有两份缓存：main.js 的 `programs`/`statuses`（详情页与操作区用）和
// 工作台的 `st.items`（侧栏列表用它渲染）。以前这里只刷前者再 `renderSidebar()`，
// 而 `renderSidebar()` 在有工作台时只是把旧的 `st.items` 重画一遍 —— 于是导入完
// 新程序要等 5 秒轮询才出现在侧栏里。现在这里一次刷两处。
async function reloadPrograms() {
  programs = await invoke("get_programs");
  statuses = (await invoke("batch_status_local")) || [];
  if (window.usWorkbench && typeof window.usWorkbench.load === "function") {
    await window.usWorkbench.load();
  } else {
    renderSidebar();
  }
}

// ---------- 导入本地模板 ----------
let importFileHandle = null;

function openImportModal() {
  document.querySelector("#import-file").value = "";
  importFileHandle = null;
  document.querySelector("#import-file").classList.remove("attached");
  document.querySelector("#import-drop").hidden = false;
  document.querySelector("#import-overwrite").checked = false;
  document.querySelector("#import-modal").hidden = false;
}

document.querySelector("#import-file").addEventListener("change", () => {
  importFileHandle = document.querySelector("#import-file").files[0] || null;
  document.querySelector("#import-drop").hidden = !!importFileHandle;
  document.querySelector("#import-file").classList.toggle("attached", !!importFileHandle);
});
document.querySelector("#import-drop").onclick = () => document.querySelector("#import-file").click();

async function doLocalImport() {
  if (!importFileHandle) {
    document.querySelector("#import-file").click();
    return;
  }
  const text = await importFileHandle.text();
  try {
    const p = await invoke("import_template_json", {
      templateJson: text,
      overwrite: document.querySelector("#import-overwrite").checked,
    });
    document.querySelector("#import-modal").hidden = true;
    showNotice(t("toast.imported", { name: p.name }));
    await reloadPrograms();
    switchCurrent(p.id);
  } catch (e) {
    showNotice(String(e), true);
  }
}

document.querySelector("#import-modal-ok").onclick = doLocalImport;

// ---------- 主题 / 语言 / 收窄 ----------
function applyTheme(theme) {
  const root = document.documentElement;
  root.dataset.theme = theme;
  const icon = theme === "dark" ? "☾" : "☀";
  document.querySelector("#theme-btn").textContent = icon;
  document.querySelector("#mob-theme").textContent = icon;
  try {
    localStorage.setItem("us-theme", theme);
  } catch {}
  invoke("set_theme", { theme }).catch(() => {});
}

function toggleTheme() {
  const root = document.documentElement;
  const cur = root.dataset.theme || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
  applyTheme(cur === "dark" ? "light" : "dark");
}

async function toggleLang() {
  const next = i18n.effective === "zh-CN" ? "en" : "zh-CN";
  try {
    const loc = await invoke("set_locale", { locale: next });
    i18n.effective = loc.effective;
    const resp = await fetch(`./locales/${i18n.effective}.json`);
    i18n.dict = (await resp.json()) || {};
    applyStaticI18n();
    renderHeader();
    renderForm();
    renderActions();
    renderSidebar();
    if (view === "library") renderLibrary();
    else if (view === "manage" && window.usWorkbench) {
      // 工作台的列表与详情都是即时渲染的，切语言后重建一次即可
      window.usWorkbench.renderList();
      const id = window.usWorkbench.selectedId();
      if (id) window.usWorkbench.select(id);
    }
  } catch (e) {
    showNotice(String(e), true);
  }
}

// ---------- 移动端独立展示（方案 B：卡片流 + 二级页面） ----------
function isMobile() {
  return window.matchMedia("(max-width: 720px)").matches;
}

let mobPage = "prog"; // prog(卡片主页) / batch / lib / log

function renderMobCards() {
  if (window.usWorkbench) return window.usWorkbench.renderMobCards();
  el.mobCards.innerHTML = "";
  if (!statuses.length) {
    const empty = document.createElement("div");
    empty.className = "mob-card mob-empty";
    empty.textContent = t("ui.empty");
    el.mobCards.appendChild(empty);
    return;
  }
  const shown = statuses;
  for (const s of shown) {
    const p = programs.find((x) => x.id === s.id);
    const card = document.createElement("div");
    card.className = "mob-card";
    const ico = document.createElement("span");
    ico.className = "mob-card-ico";
    ico.textContent = (s.name || "?").slice(0, 1).toUpperCase();
    const body = document.createElement("span");
    body.className = "mob-card-body";
    const nm = document.createElement("span");
    nm.className = "mob-card-name";
    nm.textContent = s.name;
    const st = document.createElement("span");
    st.className = "mob-card-sub";
    st.textContent = s.status.running ? "● " + t("st.running") : "○ " + (s.status.installed ? t("st.stopped") : t("st.not_installed_bare"));
    body.append(nm, st);
    const dot = document.createElement("span");
    dot.className = "mob-card-dot" + (s.status.running ? " on" : "");
    card.append(dot, ico, body);
    card.onclick = () => {
      switchCurrent(s.id);
      mobGotoDetail(p);
    };
    attachCtxLongPress(card, (x, y) => ctxMenuAt(x, y, progItemMenu(p)));
    el.mobCards.appendChild(card);
  }
}

function setMobTab(active) {
  document.querySelectorAll("#mob-tabbar .mob-tab").forEach((b) => {
    b.classList.toggle("active", b.dataset.mtab === active);
  });
}

function mobGotoDetail(p) {
  el.mobShell.hidden = true;
  el.mobBackbar.hidden = false;
  el.mobDetailTitle.textContent = p?.name || "";
  document.body.classList.add("mob-detail");
  setMobTab("prog");
  const log = el.manageLog;
  if (log.dataset.open === "1") {
    log.dataset.open = "0";
    const tg = document.querySelector("#manage-log-toggle");
    if (tg) {
      tg.textContent = "▸";
      tg.title = t("log.expand");
    }
  }
}

function mobShowHome() {
  switchView("manage");
  el.mobShell.hidden = false;
  el.mobBackbar.hidden = true;
  document.body.classList.remove("mob-detail");
  setMobTab("prog");
  el.manageView.hidden = true;
  renderMobCards();
}

function mobSyncView(v) {
  // switchView 的移动端协调层：管理二级页显示返回栏；其余视图直接显示
  if (v === "manage" && document.body.classList.contains("mob-detail")) {
    el.mobShell.hidden = true;
    el.mobBackbar.hidden = false;
    el.manageView.hidden = false;
  } else {
    el.mobShell.hidden = true;
    el.mobBackbar.hidden = true;
    el.manageView.hidden = v !== "manage";
    document.body.classList.remove("mob-detail");
  }
}

function mobSwitchPage(tab) {
  mobPage = tab;
  document.body.classList.remove("mob-detail");
  if (tab === "prog") {
    mobShowHome();
  } else if (tab === "lib") {
    switchView("library");
    setMobTab("lib");
  } else if (tab === "log") {
    openLogCenter();
    setMobTab("log");
  }
}

function setupMobile() {
  const mm = window.matchMedia("(max-width: 720px)");
  const apply = () => {
    const on = mm.matches;
    document.body.classList.toggle("mob", on);
    if (on) {
      el.mobTabbar.hidden = false;
      mobSwitchPage("prog");
    } else {
      el.mobShell.hidden = true;
      el.mobTabbar.hidden = true;
      el.mobBackbar.hidden = true;
      document.body.classList.remove("mob-detail");
    }
  };
  mm.addEventListener("change", apply);
  apply();
}

document.querySelector("#mob-tabbar").addEventListener("click", (e) => {
  const tab = e.target.closest(".mob-tab");
  if (tab) mobSwitchPage(tab.dataset.mtab);
});
document.querySelector("#mob-back").onclick = () => mobShowHome();
document.querySelector("#mob-theme").onclick = toggleTheme;
document.querySelector("#mob-lang").onclick = toggleLang;
document.querySelector("#mob-settings").onclick = openSettings;
document.querySelector("#mob-github").onclick = () => openExternal("https://github.com/Jonnyan404/universal-shell");
setupMobile();



// 布局重构后侧栏收窄为图标栏：折叠按钮与侧栏容器都不再存在，这里保留兼容判断，
// 免得旧数据目录里的残留标记把启动流程打断。
const collapseBtn = document.querySelector("#collapse-btn");
if (collapseBtn) {
  collapseBtn.onclick = () => {
    const root = document.documentElement;
    root.dataset.sidebar = root.dataset.sidebar === "narrow" ? "" : "narrow";
    collapseBtn.textContent = root.dataset.sidebar === "narrow" ? "»" : "«";
  };
}
document.querySelector("#theme-btn").onclick = toggleTheme;
document.querySelector("#lang-btn").onclick = toggleLang;
const navLogBtn = document.querySelector("#nav-log") || document.querySelector("#log-center-link");
if (navLogBtn) navLogBtn.onclick = () => openLogCenter();
const navLibBtn = document.querySelector("#nav-lib");
if (navLibBtn) navLibBtn.onclick = () => switchView("library");
const navProgBtn = document.querySelector("#nav-prog");
if (navProgBtn) navProgBtn.onclick = () => switchView("manage");
document.querySelector("#log-search").oninput = (e) => {
  logCenter.search = e.target.value;
  renderLogContent();
};
document.querySelector("#log-follow").onclick = () => {
  logCenter.follow = !logCenter.follow;
  const b = document.querySelector("#log-follow");
  b.textContent = t("log.follow") + (logCenter.follow ? " ✓" : "");
  if (logCenter.follow) startLogCenterTailing();
  else stopLogCenterTailing();
};
document.querySelector("#log-copy").onclick = async () => {
  try {
    await navigator.clipboard.writeText(logCenter.text || "");
    showNotice(t("toast.addr_copied"));
  } catch {
    showNotice(t("toast.copy_fail"), true);
  }
};
document.querySelector("#log-clear-src").onclick = async () => {
  try {
    await invoke("clear_shell_log");
    logCenter.text = "";
    renderLogContent();
  } catch (e) {
    showNotice(String(e), true);
  }
};
document.querySelector("#manage-log-copy").onclick = () => {
  // 小窗是倒序展示的，复制得还原成时间顺序，粘到别处才读得顺
  const text = [...el.manageLogContent.children]
    .map((d) => d.textContent)
    .reverse()
    .join("\n");
  navigator.clipboard.writeText(text).catch(() => {});
};
document.querySelector("#manage-log-refresh").onclick = refreshManageLog;
document.querySelector("#manage-log-open").onclick = () => {
  logCenter.kind = "program";
  logCenter.id = current?.id || "";
  openLogCenter();
};

// 底部抽屉：展开/收起 + 上下拖动调节高度（最小 80px，不超过窗口 70%）
function setupManageLogDrawer() {
  const log = document.querySelector("#manage-log");
  const handle = document.querySelector("#manage-log-resize");
  const toggle = document.querySelector("#manage-log-toggle");
  let drawerHeight = 120;
  const applyHeight = () => {
    const open = log.dataset.open === "1";
    log.style.setProperty("--ml-height", drawerHeight + "px");
    toggle.textContent = open ? "▾" : "▸";
    toggle.title = t(open ? "log.collapse" : "log.expand");
    // 倒序展示：顶部就是最新一屏，展开时回到顶部
    if (open) requestAnimationFrame(() => { el.manageLogContent.scrollTop = 0; });
  };
  toggle.onclick = () => {
    log.dataset.open = log.dataset.open === "1" ? "0" : "1";
    applyHeight();
  };
  if (handle) {
    let dragging = false;
    let startY = 0;
    let startH = 0;
    const onMove = (e) => {
      if (!dragging || log.dataset.open !== "1") return;
      const dh = e.clientY - startY;
      const maxH = window.innerHeight * 0.7;
      drawerHeight = Math.min(Math.max(startH - dh, 80), maxH);
      applyHeight();
    };
    const onUp = () => {
      dragging = false;
      document.body.classList.remove("resizing-log");
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    handle.addEventListener("mousedown", (e) => {
      e.preventDefault();
      dragging = true;
      startY = e.clientY;
      startH = log.getBoundingClientRect().height;
      document.body.classList.add("resizing-log");
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    });
  }
  applyHeight();
}
setupManageLogDrawer();

// 程序管理按钮
document.querySelector("#edit-modal-close").onclick = () => (document.querySelector("#edit-modal").hidden = true);
document.querySelector("#edit-modal-cancel").onclick = () => (document.querySelector("#edit-modal").hidden = true);
document.querySelector("#edit-modal-save").onclick = saveEdit;

// F9：本机窗口支持服务端原生选择器；远程（无该能力）隐藏按钮、手填路径
const browseBtn = document.querySelector("#edit-binary-browse");
globalThis.haveNativePick = false;
globalThis.canUpload = true;
(async () => {
  try {
    const caps = await (await fetch("/api/capabilities")).json();
    globalThis.haveNativePick = !!caps.native_pick_available;
    globalThis.canUpload = caps.upload_supported !== false;
  } catch {}
  browseBtn.hidden = !globalThis.haveNativePick;
  uploadBtn.hidden = !globalThis.canUpload;
  // 能力异步到达后，重绘当前表单/字段行，让文件/目录字段补上「浏览」按钮
  if (current) renderForm();
  if (document.querySelector("#edit-field-body").childElementCount) renderFieldRows();
})();
browseBtn.onclick = async () => {
  try {
    const r = await invoke("pick_file");
    if (r.supported && r.path) document.querySelector("#edit-binary").value = r.path;
  } catch (e) {
    showNotice(String(e), true);
  }
};

// F-9 二期：远程上传二进制 → 服务端存盘并回填绝对路径（本地程序 binary 直接引用）
const uploadBtn = document.querySelector("#edit-binary-upload");
const uploadInput = document.querySelector("#edit-binary-file");
uploadBtn.onclick = () => uploadInput.click();
uploadInput.onchange = async () => {
  const f = uploadInput.files[0];
  uploadInput.value = "";
  if (!f) return;
  const headers = {};
  if (usToken) headers["X-Universal-Token"] = usToken;
  uploadBtn.disabled = true;
  uploadBtn.textContent = Math.round(f.size / 1048576) + "M…";
  try {
    const res = await fetch("/api/upload?name=" + encodeURIComponent(f.name), {
      method: "POST",
      headers,
      body: f,
    });
    const j = await res.json();
    if (!res.ok || !j.ok) throw new Error(j.error || "upload failed: HTTP " + res.status);
    document.querySelector("#edit-binary").value = j.data.path;
    showNotice(t("toast.uploaded", { name: j.data.name }));
  } catch (e) {
    showNotice(String(e), true);
  } finally {
    uploadBtn.disabled = false;
    uploadBtn.textContent = t("act.upload");
  }
};
document.querySelector("#edit-add-field").onclick = () => {
  editing.fields.push({ key: "", kind: "string", label: "", default: "", placeholder: "", required: false });
  renderFieldRows();
};
document.querySelector("#edit-add-env").onclick = () => {
  editing.env.push({ key: "", value: "", label: "" });
  renderEnvRows();
};
document.querySelector("#import-modal-close").onclick = () => (document.querySelector("#import-modal").hidden = true);
document.querySelector("#import-modal-cancel").onclick = () => (document.querySelector("#import-modal").hidden = true);

// 删除确认弹窗：软删除（保留数据） / 连带数据彻底删除
document.querySelector("#del-modal-close").onclick = closeDelModal;
document.querySelector("#del-modal-cancel").onclick = closeDelModal;
document.querySelector("#del-modal-keep").onclick = () => applyDelete(false);
document.querySelector("#del-modal-purge").onclick = () => applyDelete(true);

// 模板库 / 设置 入口（批量与服务已并入工作台）
document.querySelector("#github-btn").onclick = () => openExternal("https://github.com/Jonnyan404/universal-shell");

// 设置模态
document.querySelector("#settings-btn").onclick = openSettings;
document.querySelector("#settings-modal-close").onclick = () => { settingsModal.hidden = true; };
document.querySelector("#sett-cancel").onclick = () => { settingsModal.hidden = true; };
document.querySelector("#settings-form").onsubmit = (e) => {
  e.preventDefault();
  saveSettings();
};
document.querySelector("#sett-proxy-enabled").onchange = (e) => {
  document.querySelector(".sett-proxy").classList.toggle("off", !e.target.checked);
  renderSettNet();
};
document.querySelector("#sett-net-type").onchange = () => { updateNetAddline(); renderSettNet(); };
document.querySelector("#sett-net-submit").onclick = () => settNetSubmit();
document.querySelector("#sett-net-ping-all").onclick = () => measureNetPings();
document.querySelector("#sett-check-update").onclick = () => checkShellUpdate(true);
document.querySelector("#sett-update-link").onclick = (e) => {
  e.preventDefault();
  const url = shellUpdate && shellUpdate.release_url;
  if (url) openExternal(url);
};
document.querySelector("#sett-token-copy").onclick = () => {
  const inp = document.querySelector("#sett-token");
  inp.select();
  inp.setSelectionRange(0, 99999);
  navigator.clipboard.writeText(inp.value).then(() => showNotice(t("toast.copied"))).catch(() => {});
};

// 随机重设访问令牌：生成本地 24 字节随机串并填充输入框（保存时经 set_web_settings 持久化）
document.querySelector("#sett-token-regen").onclick = () => {
  const bytes = new Uint8Array(24);
  crypto.getRandomValues(bytes);
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  document.querySelector("#sett-token").value = hex;
};

// 模板库
document.querySelector("#lib-import-local").onclick = openImportModal;
document.querySelector("#lib-manage-sources").onclick = openSourcesModal;
document.querySelector("#lib-cache-toggle").onclick = () => {
  if (el.libCacheDrawer) el.libCacheDrawer.hidden = !el.libCacheDrawer.hidden;
};
document.querySelector("#sources-modal-close").onclick = () => { document.querySelector("#sources-modal").hidden = true; };
document.querySelector("#sources-cancel").onclick = () => { document.querySelector("#sources-modal").hidden = true; };
document.querySelector("#sources-save").onclick = saveSources;

// 模板差异模态
document.querySelector("#template-diff-modal-close").onclick = closeTemplateDiffModal;
document.querySelector("#template-diff-cancel").onclick = closeTemplateDiffModal;
document.querySelector("#template-diff-apply").onclick = applyTemplateUpdate;

// ---------- 启动 ----------
async function boot() {
  // PWA：仅真浏览器注册 service worker（Tauri 内嵌 WebView 或旧浏览器安全跳过）
  if ("serviceWorker" in navigator && !window.__TAURI__) {
    try {
      await navigator.serviceWorker.register("sw.js");
    } catch {}
  }
  try {
    await loadLocale();
  } catch {}
  applyStaticI18n();
  // 主题：服务端持久化 > 本地缓存 > 系统偏好
  let savedTheme = null;
  try { savedTheme = localStorage.getItem("us-theme"); } catch {}
  try {
    const { theme } = await invoke("get_theme");
    if (theme && theme !== "auto") savedTheme = theme;
  } catch {}
  const dark = savedTheme ? savedTheme === "dark" : matchMedia("(prefers-color-scheme: dark)").matches;
  applyTheme(dark ? "dark" : "light");

  programs = await invoke("get_programs");
  statuses = (await invoke("batch_status_local")) || [];
  // 工作台：统一列表（程序 + 手工服务）。列表与详情都由它驱动。
  const wbItems = window.usWorkbench ? await window.usWorkbench.load() : [];
  renderSidebar();
  if (isMobile()) {
    renderMobCards();
    mobShowHome();
  } else if (wbItems.length) {
    await window.usWorkbench.select(wbItems[0].id);
  }

  registries = await invoke("get_registries");
  registryUrl = registries[0] ?? "";
  libSource = registries[0] ?? null;
  await ensureLibraryFromCache();

  // 模板库搜索框只建一次，输入时仅刷新列表（避免重建光标跳走）
  const libSearchInput = document.createElement("input");
  libSearchInput.placeholder = t("lib.search_ph");
  libSearchInput.value = libSearchValue;
  libSearchInput.oninput = () => {
    libSearchValue = libSearchInput.value;
    renderLibrary();
  };
  el.libSearch.innerHTML = "";
  el.libSearch.appendChild(libSearchInput);

  checkShellUpdate(false);
  connectWS();

  // F-3/F10：运行状态由 WS 事件推送为主；此定时器仅作 WS 断线兜底。
  // 该兜底查的是本地句柄状态（批处理 RPC 无进程派生），保持低频即可。
  // 页面隐藏（最小化/切后台）时暂停，避免恢复窗口时积压的定时任务风暴。
  setInterval(() => {
    if (!pageHidden()) refreshAllStatuses();
  }, 15000);
  setInterval(() => {
    if (!pageHidden() && current) refreshManageLog();
  }, 3000);
}

// 深链接（?view=services 等）放在 finally：启动过程中 switchCurrent 会把视图
// 切回管理页，早切一定被抢走；而 boot 中途出错（如模板源不可达）也要能落到目标页，
// 所以不能只写在 boot 正常流程的末尾。
boot()
  .catch((e) => {
    showNotice(String(e), true);
  })
  .finally(() => {
    const deep = new URLSearchParams(location.search).get("view");
    if (deep && ["library", "log"].includes(deep)) {
      switchView(deep);
      return;
    }
    // 工作台首次进入：桌面端默认选中第一项（boot 早期可能还拿不到 usWorkbench）
    if (window.usWorkbench) {
      const items = window.usWorkbench.items();
      // ?sel=<id> 可直达某一条（分享 / 刷新后回到原处）
      const want = new URLSearchParams(location.search).get("sel");
      if (want && items.some((i) => i.id === want)) {
        window.usWorkbench.select(want);
      } else if (!isMobile() && items.length && !window.usWorkbench.selectedId()) {
        window.usWorkbench.select(items[0].id);
      }
    }
  });





