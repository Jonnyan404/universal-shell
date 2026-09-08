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
const templateStatusCache = {};
const importing = new Set();
const installing = new Set();
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
  manageLog: document.querySelector("#manage-log"),
  manageLogContent: document.querySelector("#manage-log-content"),
  envBody: document.querySelector("#env-body"),
  envSection: document.querySelector("#env-section"),
  manageView: document.querySelector("#manage-view"),
  batchView: document.querySelector("#batch-view"),
  libraryView: document.querySelector("#library-view"),
  batchBody: document.querySelector("#batch-body"),
  batchTable: document.querySelector(".batch-table"),
  batchCards: document.querySelector("#batch-cards"),
  batchCheckedAt: document.querySelector("#batch-checked-at"),
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
  setTimeout(() => box.remove(), 3200);
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
  el.tabs.innerHTML = "";
  statuses.forEach((s) => {
    if (s.hidden) return;
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
    { label: p.hidden ? t("act.unhide") : t("act.hide"), onClick: () => toggleProgramHidden(p) },
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

// 侧栏空白区右键 = 新建（行右键已自行 stopPropagation，不会冒泡到这里）。
// 只排除程序条目本身，nav 内空白（如列表下方空区）也应弹出菜单。
document.querySelector(".sidebar").addEventListener("contextmenu", (e) => {
  if (
    !e.target.closest(".prog-item") &&
    !e.target.closest("#ctx-menu") &&
    !e.target.closest(".modal")
  ) {
    e.preventDefault();
    ctxMenuAt(e.clientX, e.clientY, [
      { label: t("menu.new"), onClick: () => openEditModal(null) },
    ]);
  }
});
attachCtxLongPress(document.querySelector(".sidebar"), (x, y) => {
  const hit = document.elementFromPoint(x, y);
  if (hit && hit.closest("button, .prog-item, #ctx-menu, .modal")) return;
  ctxMenuAt(x, y, [{ label: t("menu.new"), onClick: () => openEditModal(null) }]);
});

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

async function toggleProgramHidden(p) {
  try {
    await invoke("set_program_hidden", { programId: p.id, hidden: !p.hidden });
    showNotice(t(p.hidden ? "toast.unhidden" : "toast.hidden", { name: p.name }));
    await reloadPrograms();
  } catch (e) {
    showNotice(String(e), true);
  }
}

async function confirmAndDelete(p) {
  if (!confirm(t("confirm.delete", { name: p.name }))) return;
  try {
    await invoke("delete_program", { programId: p.id });
    showNotice(t("toast.deleted", { name: p.name }));
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

// ---------- 主视图 ----------
function switchView(v) {
  view = v;
  el.manageView.hidden = v !== "manage";
  el.batchView.hidden = v !== "batch";
  el.libraryView.hidden = v !== "library";
  el.logView.hidden = v !== "log";
  const dlBtn = document.querySelector("#dl-btn");
  if (dlBtn) dlBtn.hidden = v !== "manage";
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
    if (v === "batch") refreshBatchLocal();
    else if (v === "library") {
      if (!manifest) ensureLibraryFromCache();
      else renderLibrary();
    }
  }
  if (v !== "log") stopLogCenterTailing();
  renderSidebar();
  document.querySelectorAll(".sidebar-foot .sidebar-settings").forEach((b) => {
    b.classList.toggle("active", b.id === (v === "batch" ? "batch-link" : v === "library" ? "library-link" : v === "log" ? "log-center-link" : ""));
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
  const hint = document.querySelector("#manage-log-hint");
  if (hint) hint.textContent = current.id;
  refreshManageLog();
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
function scheduleAutosave() {
  if (!current) return;
  const id = current.id;
  const snap = { ...values };
  clearTimeout(autosaveTimers[id]);
  autosaveTimers[id] = setTimeout(async () => {
    try {
      await invoke("save_values", { programId: id, values: snap });
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
  // 按钮组整体居中；启停/重启 与 图标组 之间用分隔条隔开
  const group1 = document.createElement("span");
  group1.className = "act-group";

  const start = document.createElement("button");
  start.id = "start-btn";
  start.textContent = "▶ " + t("act.start");
  start.onclick = async () => {
    try {
      const st = await invoke("start_program", { programId: current.id, values });
      applyStatus(st);
      showNotice(t("toast.started"));
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
      showNotice(t("toast.restarted"));
      refreshManageLog();
    } catch (e) {
      showNotice(String(e), true);
    }
  };
  group1.appendChild(restart);
  actions.appendChild(group1);

  // 下载/更新按钮已迁入状态栏（对齐 Tauri），这里只接上绑定
  const dl = document.querySelector("#dl-btn");
  if (dl) {
    dl.dataset.programId = current.id;
    dl.dataset.installed = statuses.find((s) => s.id === current.id)?.status?.installed ? "1" : "0";
    dl.onclick = () => installProgram(current.id, dl);
  }

  // 图标组（打开目录/复制地址/打开网站）：仅在存在时插入分隔条
  const group2 = document.createElement("span");
  group2.className = "act-group act-icons";
  const pushIcon = (b) => {
    group2.appendChild(b);
  };
  if (current.repo) {
    const appDir = document.createElement("button");
    appDir.className = "icon-btn ops-dir";
    appDir.title = t("act.open_app_dir");
    appDir.textContent = "📁";
    appDir.onclick = async () => {
      try {
        await invoke("reveal_app_dir", { programId: current.id });
      } catch (e) {
        showNotice(String(e), true);
      }
    };
    pushIcon(appDir);
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
  const editBtn = document.createElement("button");
  editBtn.className = "icon-btn ops-edit";
  editBtn.title = t("act.edit");
  editBtn.textContent = "✎";
  editBtn.onclick = () => openEditModal(current);
  pushIcon(editBtn);
  const delBtn = document.createElement("button");
  delBtn.className = "icon-btn ops-del";
  delBtn.title = t("act.delete");
  delBtn.textContent = "🗑";
  delBtn.onclick = () => confirmAndDelete(current);
  pushIcon(delBtn);
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
  renderButtons(st);
  syncInstallBtns(current.id);
  renderSidebar();
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
    if (view === "batch") await refreshBatchLocal();
    else if (current?.id === id) await refreshStatusLocal();
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
        refreshAllStatuses();
        if (current) refreshManageLog();
      } else if (m && m.type === "log-tick") {
        // F-3/F11：日志增量推送提示 → 对当前程序做一次增量 tail 拉取
        if (current && m.programId === current.id) refreshManageLog();
      }
    } catch {}
  };
  ws.onclose = () => setTimeout(connectWS, 2000);
}

// ---------- 批量管理 ----------
async function refreshBatchLocal() {
  try {
    statuses = await invoke("batch_status_local");
    renderSidebar();
    renderBatch();
  } catch (e) {
    showNotice(String(e), true);
  }
}

async function checkUpdates() {
  const btn = document.querySelector("#batch-check-updates");
  if (btn?.classList.contains("busy")) return;
  if (btn) {
    btn.classList.add("busy");
    btn.disabled = true;
    btn.textContent = t("dl.checking_short");
  }
  try {
    const full = await invoke("batch_status");
    statuses = full;
    renderSidebar();
    renderBatch();
    showNotice(t("dl.done"));
  } catch (e) {
    showNotice(String(e), true);
  } finally {
    if (btn) {
      btn.classList.remove("busy");
      btn.disabled = false;
      btn.textContent = t("batch.check");
    }
  }
}

function renderBatch() {
  const checkedAt = statuses.reduce((m, it) => {
    const t0 = it.status?.latest_checked_at;
    return t0 && t0 > m ? t0 : m;
  }, 0);
  el.batchCheckedAt.textContent = checkedAt
    ? t("check.last_checked", { ago: timeAgo(checkedAt) })
    : t("check.not_checked");

  const mobile = isMobile();
  el.batchTable.hidden = mobile;
  el.batchCards.hidden = !mobile;
  if (mobile) { renderBatchMobile(); return; }

  el.batchBody.innerHTML = "";
  if (!statuses.length) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 6;
    td.textContent = t("side.empty");
    tr.appendChild(td);
    el.batchBody.appendChild(tr);
    return;
  }
  for (const item of statuses) {
    const s = item.status;
    const tr = document.createElement("tr");
    const tdName = document.createElement("td");
    tdName.textContent = item.name;
    if (item.hidden) {
      const tag = document.createElement("span");
      tag.className = "batch-hidden";
      tag.textContent = t("st.hidden");
      tdName.appendChild(tag);
    }
    tr.appendChild(tdName);
    const tdLocal = document.createElement("td");
    tdLocal.textContent = s.local_version || "—";
    tr.appendChild(tdLocal);
    const tdLatest = document.createElement("td");
    tdLatest.textContent = s.latest_version ?? t("st.unknown");
    tr.appendChild(tdLatest);
    const tdState = document.createElement("td");
    tdState.textContent = !s.installed ? t("st.not_installed_bare") : s.running ? t("st.running") : t("st.stopped");
    tdState.className = !s.installed ? "missing" : s.running ? "running" : "stopped";
    tr.appendChild(tdState);
    const tdAuto = document.createElement("td");
    const auto = document.createElement("input");
    auto.type = "checkbox";
    auto.checked = s.autostart;
    auto.addEventListener("change", async () => {
      auto.disabled = true;
      try {
        await invoke("set_autostart", { programId: item.id, enabled: auto.checked });
        showNotice(t("toast.autostart_updated", { name: item.name }));
      } catch (e) {
        auto.checked = !auto.checked;
        showNotice(String(e), true);
      } finally {
        auto.disabled = false;
      }
    });
    tdAuto.appendChild(auto);
    tr.appendChild(tdAuto);
    const tdHide = document.createElement("td");
    const hide = document.createElement("input");
    hide.type = "checkbox";
    hide.checked = item.hidden;
    hide.addEventListener("change", async () => {
      hide.disabled = true;
      try {
        await invoke("set_program_hidden", { programId: item.id, hidden: hide.checked });
        showNotice(hide.checked ? t("toast.hidden", { name: item.name }) : t("toast.unhidden", { name: item.name }));
        programs = await invoke("get_programs");
        const hidCurrent = item.id === current?.id && hide.checked;
        if (hidCurrent) current = null;
        if (!current) current = programs.find((p) => !p.hidden) || null;
        await refreshBatchLocal();
        if (hidCurrent && current) await switchCurrent(current.id);
      } catch (e) {
        hide.checked = !hide.checked;
        showNotice(String(e), true);
      } finally {
        hide.disabled = false;
      }
    });
    tdHide.appendChild(hide);
    tr.appendChild(tdHide);
    el.batchBody.appendChild(tr);

    const ops = batchOpButtons(item);
    const opsRow = document.createElement("tr");
    opsRow.className = "batch-ops-row";
    const opsCell = document.createElement("td");
    opsCell.colSpan = 6;
    opsCell.appendChild(ops);
    opsRow.appendChild(opsCell);
    el.batchBody.appendChild(opsRow);
  }
}

function hasRemoteSource(item) {
  return !!(item.repo || statusSource(item.id));
}

function batchIconBtn(ico, titleKey, cls, fn) {
  const b = document.createElement("button");
  b.className = "icon-btn" + (cls ? " " + cls : "");
  b.title = t(titleKey);
  b.textContent = ico;
  b.onclick = fn;
  return b;
}

function batchDownloadBtn(item, s) {
  const isUpToDate = s.installed && s.up_to_date;
  const dl = document.createElement("button");
  dl.className = "icon-btn" + (isUpToDate ? " ops-ok" : " ops-dl");
  dl.dataset.programId = item.id;
  dl.dataset.installed = s.installed ? "1" : "0";
  dl.title = isUpToDate ? t("st.latest") : s.installed ? t("dl.update") : t("dl.download");
  dl.textContent = isUpToDate ? "✓" : "⇩";
  dl.disabled = isUpToDate;
  dl.onclick = () => installProgram(item.id, dl);
  return dl;
}

async function batchAct(item, rpc) {
  try {
    if (rpc !== "stop_program") {
      const vals = (await invoke("get_values", { programId: item.id }).catch(() => ({}))) || {};
      await invoke(rpc, { programId: item.id, values: vals });
    } else {
      await invoke(rpc, { programId: item.id });
    }
    await refreshBatchLocal();
  } catch (e) { showNotice(String(e), true); }
}

function batchOpButtons(item) {
  const s = item.status;
  const ops = document.createElement("span");
  ops.className = "batch-ops";
  if (hasRemoteSource(item)) ops.appendChild(batchDownloadBtn(item, s));
  ops.appendChild(batchIconBtn("▶", "act.start", "ops-start", () => batchAct(item, "start_program")));
  ops.appendChild(batchIconBtn("↻", "act.restart", "ops-restart", () => batchAct(item, "restart_program")));
  ops.appendChild(batchIconBtn("■", "act.stop", "ops-stop", () => batchAct(item, "stop_program")));
  if (item.repo) ops.appendChild(batchIconBtn("📁", "act.open_app_dir", "ops-dir", async () => {
    try { await invoke("reveal_app_dir", { programId: item.id }); }
    catch (e) { showNotice(String(e), true); }
  }));
  ops.appendChild(batchIconBtn("✎", "act.edit", "ops-edit", () => openEditModal(programs.find((x) => x.id === item.id))));
  ops.appendChild(batchIconBtn("🗑", "act.delete", "ops-del", () => confirmAndDelete(programs.find((x) => x.id === item.id))));
  return ops;
}

// 移动端：批量管理改卡片列表流（G3）
function renderBatchMobile() {
  el.batchCards.innerHTML = "";
  if (!statuses.length) {
    const empty = document.createElement("div");
    empty.className = "mb-card mb-empty";
    empty.textContent = t("side.empty");
    el.batchCards.appendChild(empty);
    return;
  }
  for (const item of statuses) {
    const s = item.status;
    const card = document.createElement("div");
    card.className = "mb-card";
    const top = document.createElement("div");
    top.className = "mb-card-top";
    const name = document.createElement("span");
    name.className = "mb-card-name";
    name.textContent = item.name;
    if (item.hidden) {
      const tag = document.createElement("span");
      tag.className = "batch-hidden";
      tag.textContent = t("st.hidden");
      name.appendChild(tag);
    }
    const st = document.createElement("span");
    st.className = "mb-state" + (!s.installed ? " missing" : s.running ? " running" : " stopped");
    st.textContent = !s.installed ? t("st.not_installed_bare") : s.running ? t("st.running") : t("st.stopped");
    top.append(name, st);
    const ver = document.createElement("div");
    ver.className = "mb-card-vers";
    const lv = document.createElement("span");
    lv.textContent = `${t("th.local_ver")}: ${s.local_version || "—"}`;
    const uv = document.createElement("span");
    uv.textContent = `${t("th.latest_ver")}: ${s.latest_version ?? t("st.unknown")}`;
    ver.append(lv, uv);
    const toggles = document.createElement("div");
    toggles.className = "mb-card-toggles";
    const mkToggle = (label, checked, fn) => {
      const lab = document.createElement("label");
      lab.className = "mb-toggle";
      const box = document.createElement("input");
      box.type = "checkbox";
      box.checked = checked;
      lab.append(box, document.createTextNode(label));
      box.addEventListener("change", fn);
      return lab;
    };
    toggles.appendChild(mkToggle(t("th.autostart"), s.autostart, async (ev) => {
      const box = ev.target;
      box.disabled = true;
      try {
        await invoke("set_autostart", { programId: item.id, enabled: box.checked });
        showNotice(t("toast.autostart_updated", { name: item.name }));
      } catch (e) {
        box.checked = !box.checked;
        showNotice(String(e), true);
      } finally { box.disabled = false; }
    }));
    toggles.appendChild(mkToggle(t("th.hidden"), item.hidden, async (ev) => {
      const box = ev.target;
      box.disabled = true;
      try {
        await invoke("set_program_hidden", { programId: item.id, hidden: box.checked });
        showNotice(box.checked ? t("toast.hidden", { name: item.name }) : t("toast.unhidden", { name: item.name }));
        programs = await invoke("get_programs");
        if (item.id === current?.id && box.checked) current = null;
        if (!current) current = programs.find((p) => !p.hidden) || null;
        await refreshBatchLocal();
        if (current) await switchCurrent(current.id);
      } catch (e) {
        box.checked = !box.checked;
        showNotice(String(e), true);
      } finally { box.disabled = false; }
    }));
    const ops = batchOpButtons(item);
    card.append(top, ver, toggles, ops);
    el.batchCards.appendChild(card);
  }
}

// 是否有远程源（含 HTTP 直链）：从已加载 programs 取当前行的 source 判断
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
  } catch (e) {
    const msg = t("toast.manifest_fail", { err: e });
    el.libStatus.textContent = msg;
    showNotice(String(e), true);
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

  const pages = Math.max(1, Math.ceil(rows.length / LIB_PAGE_SIZE));
  if (libPage >= pages) libPage = pages - 1;
  const slice = rows.slice(libPage * LIB_PAGE_SIZE, (libPage + 1) * LIB_PAGE_SIZE);

  el.libList.innerHTML = "";
  if (!slice.length) el.libList.innerHTML = '<div class="lib-desc">' + t("lib.no_match") + "</div>";
  for (const [id, tpl, base] of slice) {
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
    el.libList.appendChild(card);
  }
  renderPager(pages);
  renderLocalTemplates();
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
    ops.className = "batch-ops";
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
  programs = await invoke("get_programs");
  try {
    statuses = await invoke("batch_status_local");
  } catch {}
  renderSidebar();
  if (!current || !programs.some((p) => p.id === current.id)) await switchCurrent(id);
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

const settingsModal = document.querySelector("#settings-modal");
function openSettings() {
  const acc = document.querySelector("#sett-accelerate");
  const hostEl = document.querySelector("#sett-proxy-host");
  const typeEl = document.querySelector("#sett-proxy-type");
  const userEl = document.querySelector("#sett-proxy-user");
  const passEl = document.querySelector("#sett-proxy-pass");
  acc.value = "";
  hostEl.value = "";
  userEl.value = "";
  passEl.value = "";
  typeEl.value = "http";
  invoke("get_proxy")
    .then((p) => {
      acc.value = p.accelerate_prefix || "";
      const parsed = parseProxy(p.http_proxy || "");
      typeEl.value = parsed.type;
      hostEl.value = parsed.host;
      userEl.value = parsed.user;
      passEl.value = parsed.pass;
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

async function saveSettings() {
  const acc = document.querySelector("#sett-accelerate").value;
  const type = document.querySelector("#sett-proxy-type").value;
  const host = document.querySelector("#sett-proxy-host").value.trim();
  const user = document.querySelector("#sett-proxy-user").value.trim();
  const pass = document.querySelector("#sett-proxy-pass").value;
  const hp = buildProxy(type, host, user, pass);
  const shellAuto = document.querySelector("#sett-shell-auto").checked;
  const webPort = Number(document.querySelector("#sett-web-port").value) || 0;
  const webBind = document.querySelector("#sett-lan").checked ? "0.0.0.0" : "";
  try {
    await invoke("set_proxy", { acceleratePrefix: acc, httpProxy: hp });
    try {
      await invoke("set_shell_autostart", { enabled: shellAuto });
    } catch (e) {
      showNotice(t("toast.shell_autostart_fail", { err: e }), true);
    }
    try {
      await invoke("set_web_settings", { bind: webBind, port: webPort });
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
// F-3 管理页小日志窗：整段重渲时直接滚到最新一屏
// （日志按会话追加且只展示尾部 64KB，不滚到底会在中间开始，看不到最新输出）
function renderLogBody(container, text) {
  container.innerHTML = "";
  for (const line of text.replace(/\r\n?/g, "\n").split("\n")) {
    const isErr = line.startsWith("\u001f");
    const clean = isErr ? line.slice(1) : line;
    const div = document.createElement("div");
    div.className = isErr ? "log-err" : "";
    div.textContent = clean;
    container.appendChild(div);
  }
  container.scrollTop = container.scrollHeight;
}

// F-3/F11：增量日志尾随。按程序记录上次 EOF 偏移；文件被截断时 reset 整体重渲。
const logOffsets = {};
function appendLogText(container, delta) {
  for (const line of delta.replace(/\r\n?/g, "\n").split("\n")) {
    const isErr = line.startsWith("\u001f");
    const clean = isErr ? line.slice(1) : line;
    const div = document.createElement("div");
    div.className = isErr ? "log-err" : "";
    div.textContent = clean;
    container.appendChild(div);
  }
  const atBottom = container.scrollTop + container.clientHeight >= container.scrollHeight - 60;
  if (atBottom) container.scrollTop = container.scrollHeight;
}

async function refreshManageLog() {
  if (!current) return;
  try {
    const hasOffset = Object.prototype.hasOwnProperty.call(logOffsets, current.id);
    const args = { programId: current.id };
    if (hasOffset) args.offset = logOffsets[current.id];
    const res = await invoke("get_logs", args);
    logOffsets[current.id] = res.offset;
    const box = el.manageLogContent;
    if (!hasOffset || res.reset) renderLogBody(box, res.text);
    else if (res.text) appendLogText(box, res.text);
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
  } else {
    try {
      const args = { programId: id };
      if (!reset && logCenterOffsets[id] != null) args.offset = logCenterOffsets[id];
      const res = await invoke("get_logs", args);
      if (reset || res.reset) logCenter.text = res.text;
      else logCenter.text += res.text;
      logCenterOffsets[id] = res.offset;
    } catch {
      if (reset) logCenter.text = t("shell_log.empty");
    }
  }
  logCenter.kind = kind;
  logCenter.id = id;
  el.logClearSrcBtn.hidden = kind !== "shell";
  renderLogContent();
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
    row.append(k, lab, kind, def, ph, req, del);
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

async function reloadPrograms() {
  programs = await invoke("get_programs");
  statuses = (await invoke("batch_status_local")) || [];
  renderSidebar();
}

async function duplicateCurrent() {
  if (!current) return;
  try {
    const copy = await invoke("duplicate_program", { programId: current.id });
    showNotice(t("toast.duplicated", { name: copy.name }));
    await reloadPrograms();
    switchCurrent(copy.id);
  } catch (e) {
    showNotice(String(e), true);
  }
}

async function deleteCurrent() {
  if (!current) return;
  if (!confirm(t("ui.confirm_delete", { name: current.name }))) return;
  try {
    await invoke("delete_program", { programId: current.id });
    showNotice(t("toast.deleted", { name: current.name }));
    await reloadPrograms();
    const next = programs[0];
    if (next) switchCurrent(next.id);
    else document.querySelector("#manage-view").hidden = true;
  } catch (e) {
    showNotice(String(e), true);
  }
}

async function toggleHidden() {
  if (!current) return;
  try {
    await invoke("set_program_hidden", { programId: current.id, hidden: !current.hidden });
    showNotice(t(!current.hidden ? "toast.hidden" : "toast.unhidden", { name: current.name }));
    await reloadPrograms();
  } catch (e) {
    showNotice(String(e), true);
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
    if (view === "batch") renderBatch();
    else if (view === "library") renderLibrary();
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
  el.mobCards.innerHTML = "";
  if (!statuses.length) {
    const empty = document.createElement("div");
    empty.className = "mob-card mob-empty";
    empty.textContent = t("ui.empty");
    el.mobCards.appendChild(empty);
    return;
  }
  const shown = statuses.filter((s) => !programs.find((p) => p.id === s.id)?.hidden);
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
    card.append(ico, body, dot);
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
  } else if (tab === "batch") {
    switchView("batch");
    setMobTab("batch");
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

document.querySelector("#collapse-btn").onclick = () => {
  const root = document.documentElement;
  root.dataset.sidebar = root.dataset.sidebar === "narrow" ? "" : "narrow";
  document.querySelector("#collapse-btn").textContent = root.dataset.sidebar === "narrow" ? "»" : "«";
};
document.querySelector("#theme-btn").onclick = toggleTheme;
document.querySelector("#lang-btn").onclick = toggleLang;
document.querySelector("#log-center-link").onclick = () => openLogCenter();
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
  navigator.clipboard.writeText(el.manageLogContent.textContent).catch(() => {});
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
    if (open) requestAnimationFrame(() => { el.manageLogContent.scrollTop = el.manageLogContent.scrollHeight; });
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

// 批量 / 模板库 / 设置 入口
document.querySelector("#batch-link").onclick = () => switchView("batch");
document.querySelector("#library-link").onclick = () => switchView("library");
document.querySelector("#batch-refresh").onclick = refreshBatchLocal;
document.querySelector("#batch-check-updates").onclick = checkUpdates;
document.querySelector("#batch-stop-all").onclick = async () => {
  try {
    await invoke("stop_all");
    showNotice(t("toast.stop_all"));
    await refreshBatchLocal();
  } catch (e) {
    showNotice(String(e), true);
  }
};
document.querySelector("#github-btn").onclick = () => openExternal("https://github.com/Jonnyan404/universal-shell");

// 设置模态
document.querySelector("#settings-btn").onclick = openSettings;
document.querySelector("#settings-modal-close").onclick = () => { settingsModal.hidden = true; };
document.querySelector("#sett-cancel").onclick = () => { settingsModal.hidden = true; };
document.querySelector("#settings-form").onsubmit = (e) => {
  e.preventDefault();
  saveSettings();
};
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
  try {
    await loadLocale();
  } catch {}
  applyStaticI18n();
  // 主题：优先用上次选择，未选择过才跟随系统
  let savedTheme = null;
  try {
    savedTheme = localStorage.getItem("us-theme");
  } catch {}
  const dark = savedTheme ? savedTheme === "dark" : matchMedia("(prefers-color-scheme: dark)").matches;
  applyTheme(dark ? "dark" : "light");

  programs = await invoke("get_programs");
  statuses = (await invoke("batch_status_local")) || [];
  renderSidebar();
  if (isMobile()) renderMobCards();
  if (programs.length) {
    await switchCurrent(programs[0].id);
  }
  if (isMobile()) mobShowHome();

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

  // F-3/F10：运行状态改由 WS 事件推送；此处仅作 WS 断线兜底
  setInterval(refreshAllStatuses, 15000);
  setInterval(() => {
    if (current) refreshManageLog();
  }, 3000);
}

boot().catch((e) => {
  showNotice(String(e), true);
});
