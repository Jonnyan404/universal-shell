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
  batchCheckedAt: document.querySelector("#batch-checked-at"),
  libSourceBar: document.querySelector("#lib-source-bar"),
  libFetchInfo: document.querySelector("#lib-fetch-info"),
  libStatus: document.querySelector("#lib-status"),
  libCacheDrawer: document.querySelector("#lib-cache-drawer"),
  libCacheToggle: document.querySelector("#lib-cache-toggle"),
  libSearch: document.querySelector("#lib-search"),
  libList: document.querySelector("#lib-list"),
  libPager: document.querySelector("#lib-pager"),
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

// ---------- 侧栏 ----------
function renderSidebar(preferId) {
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
    sub.textContent = s.status.running ? "● " + t("st.running") : "○ " + t( s.status.installed ? "st.stopped" : "st.not_installed");
    info.appendChild(name);
    info.appendChild(sub);
    it.appendChild(dot);
    it.appendChild(ico);
    it.appendChild(info);
    it.onclick = () => switchCurrent(s.id);
    el.tabs.appendChild(it);
  });
  if (preferId && programs.some((p) => p.id === preferId)) {
    const target = el.tabs.querySelector(`.prog-item[data-id="${preferId}"]`);
    target && target.scrollIntoView({ block: "nearest" });
  }
}

// ---------- 主视图 ----------
function switchView(v) {
  view = v;
  el.manageView.hidden = v !== "manage";
  el.batchView.hidden = v !== "batch";
  el.libraryView.hidden = v !== "library";
  if (v === "manage") {
    if (current) {
      el.progTitle.textContent = current.name;
      el.progSub.innerHTML = "";
    }
    el.progDesc.hidden = true;
  } else {
    el.progTitle.textContent = v === "batch" ? t("batch.title") : t("lib.title");
    el.progSub.innerHTML = "";
    el.progDesc.hidden = true;
    el.chips.innerHTML = "";
    if (v === "batch") refreshBatchLocal();
    else {
      if (!manifest) ensureLibraryFromCache();
      else renderLibrary();
    }
  }
  renderSidebar();
}

async function switchCurrent(id) {
  switchView("manage");
  current = programs.find((p) => p.id === id) || null;
  if (!current) return;
  values = {};
  renderHeader();
  renderForm();
  renderActions();
  await refreshStatusLocal();
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
      window.open("https://github.com/" + repo, "_blank");
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
    if (f.kind === "autostart") continue;
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
  actions.appendChild(start);

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
  actions.appendChild(stop);

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
  actions.appendChild(restart);

  const st = statuses.find((s) => s.id === current.id)?.status;
  if (current.repo || current.http_enabled) {
    const dl = document.createElement("button");
    dl.id = "dl-btn";
    dl.className = "op-btn dl-btn";
    dl.dataset.programId = current.id;
    dl.dataset.installed = st?.installed ? "1" : "0";
    dl.textContent = st?.installed
      ? st?.up_to_date ? t("st.latest") : t("dl.update")
      : t("dl.download");
    dl.disabled = !!(st?.installed && st?.up_to_date);
    dl.onclick = () => installProgram(current.id, dl);
    actions.appendChild(dl);
  }

  const edit = document.createElement("button");
  edit.className = "icon-btn";
  edit.title = t("act.edit");
  edit.textContent = "✎";
  edit.onclick = () => openEditModal(current);
  actions.appendChild(edit);

  const dup = document.createElement("button");
  dup.className = "icon-btn";
  dup.title = t("menu.copy");
  dup.textContent = "⧉";
  dup.onclick = duplicateCurrent;
  actions.appendChild(dup);

  const hide = document.createElement("button");
  hide.className = "icon-btn";
  hide.title = t(current.hidden ? "act.unhide" : "act.hide");
  hide.textContent = current.hidden ? "👁" : "🙈";
  hide.onclick = toggleHidden;
  actions.appendChild(hide);

  const del = document.createElement("button");
  del.className = "icon-btn";
  del.title = t("act.delete");
  del.textContent = "🗑";
  del.onclick = deleteCurrent;
  actions.appendChild(del);
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
    mk(t("st.not_installed"));
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
  if (!st?.installed) {
    dl.disabled = busy;
    dl.textContent = busy ? t("dl.downloading") : t("dl.download");
  } else if (st?.up_to_date) {
    dl.disabled = true;
    dl.textContent = t("st.latest");
  } else {
    dl.disabled = busy;
    dl.textContent = busy ? t("dl.downloading") : t("dl.update");
  }
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
  el.batchBody.innerHTML = "";
  const checkedAt = statuses.reduce((m, it) => {
    const t0 = it.status?.latest_checked_at;
    return t0 && t0 > m ? t0 : m;
  }, 0);
  el.batchCheckedAt.textContent = checkedAt
    ? t("check.last_checked", { ago: timeAgo(checkedAt) })
    : t("check.not_checked");
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
        if (item.id === current?.id && hide.checked) current = null;
        if (!current) current = programs.find((p) => !p.hidden) || null;
        await refreshBatchLocal();
        if (current) await switchCurrent(current.id);
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

    const ops = document.createElement("span");
    ops.className = "batch-ops";
    const hasRemote = !!(item.repo || statusSource(item.id));
    if (hasRemote) {
      const isUpToDate = s.installed && s.up_to_date;
      const dl = document.createElement("button");
      dl.className = "op-btn";
      dl.dataset.programId = item.id;
      dl.dataset.installed = s.installed ? "1" : "0";
      dl.textContent = s.installed ? (isUpToDate ? t("st.latest") : t("dl.update")) : t("dl.download");
      dl.disabled = isUpToDate;
      dl.onclick = () => installProgram(item.id, dl);
      ops.appendChild(dl);
    }
    const mkOp = (label, fn) => {
      const b = document.createElement("button");
      b.className = "op-btn";
      b.textContent = label;
      b.onclick = fn;
      return b;
    };
    ops.appendChild(mkOp(t("act.start"), async () => {
      try { await invoke("start_program", { programId: item.id, values: {} }); await refreshBatchLocal(); }
      catch (e) { showNotice(String(e), true); }
    }));
    ops.appendChild(mkOp(t("act.restart"), async () => {
      try { await invoke("restart_program", { programId: item.id, values: {} }); await refreshBatchLocal(); }
      catch (e) { showNotice(String(e), true); }
    }));
    ops.appendChild(mkOp(t("act.stop"), async () => {
      try { await invoke("stop_program", { programId: item.id }); await refreshBatchLocal(); }
      catch (e) { showNotice(String(e), true); }
    }));
    const opsRow = document.createElement("tr");
    opsRow.className = "batch-ops-row";
    const opsCell = document.createElement("td");
    opsCell.colSpan = 6;
    opsCell.appendChild(ops);
    opsRow.appendChild(opsCell);
    el.batchBody.appendChild(opsRow);
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
function renderLogBody(container, text) {
  container.innerHTML = "";
  for (const line of text.replace(/\r\n?/g, "\n").split("\n")) {
    const div = document.createElement("div");
    const isErr = line.startsWith("\u001f");
    const clean = isErr ? line.slice(1) : line;
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
    const { text } = await invoke("get_logs", { programId: current.id });
    renderLogBody(el.manageLogContent, text);
  } catch {
    /* 无日志文件或读取失败：保持现状 */
  }
}

// ---------- 壳日志 ----------
function openShellLog() {
  const modal = document.querySelector("#shell-log-modal");
  modal.hidden = false;
  const content = document.querySelector("#shell-log-content");
  const refresh = async () => {
    try {
      const text = await invoke("get_shell_log");
      renderLogBody(content, text);
    } catch (e) {
      showNotice(String(e), true);
    }
  };
  refresh();
  modal.querySelector("#shell-log-refresh").onclick = refresh;
  modal.querySelector("#shell-log-clear").onclick = async () => {
    try {
      await invoke("clear_shell_log");
      content.innerHTML = "";
    } catch (e) {
      showNotice(String(e), true);
    }
  };
  modal.querySelector("#shell-log-modal-close").onclick = () => {
    modal.hidden = true;
  };
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
    for (const kd of ["string", "boolean", "file", "directory", "autostart"]) {
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

document.querySelector("#import-btn").onclick = openImportModal;
document.querySelector("#import-file").addEventListener("change", () => {
  importFileHandle = document.querySelector("#import-file").files[0] || null;
  document.querySelector("#import-drop").hidden = !!importFileHandle;
  document.querySelector("#import-file").classList.toggle("attached", !!importFileHandle);
});
document.querySelector("#import-drop").onclick = () => document.querySelector("#import-file").click();

async function doImport() {
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

document.querySelector("#import-modal-ok").onclick = doImport;

// ---------- 主题 / 语言 / 收窄 ----------
function toggleTheme() {
  const root = document.documentElement;
  const cur = root.dataset.theme || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
  const next = cur === "dark" ? "light" : "dark";
  root.dataset.theme = next;
  const btn = document.querySelector("#theme-btn");
  btn.textContent = next === "dark" ? "☾" : "☀";
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

document.querySelector("#theme-btn").onclick = toggleTheme;
document.querySelector("#lang-btn").onclick = toggleLang;
document.querySelector("#collapse-btn").onclick = () => {
  const root = document.documentElement;
  root.dataset.sidebar = root.dataset.sidebar === "narrow" ? "" : "narrow";
  document.querySelector("#collapse-btn").textContent = root.dataset.sidebar === "narrow" ? "»" : "«";
};
document.querySelector("#shell-log-link").onclick = openShellLog;
document.querySelector("#manage-log-copy").onclick = () => {
  navigator.clipboard.writeText(el.manageLogContent.textContent).catch(() => {});
};
document.querySelector("#manage-log-refresh").onclick = refreshManageLog;

// 程序管理按钮
document.querySelector("#new-btn").onclick = () => openEditModal(null);
document.querySelector("#edit-modal-close").onclick = () => (document.querySelector("#edit-modal").hidden = true);
document.querySelector("#edit-modal-cancel").onclick = () => (document.querySelector("#edit-modal").hidden = true);
document.querySelector("#edit-modal-save").onclick = saveEdit;

// F9：本机窗口支持服务端原生选择器；远程（无该能力）隐藏按钮、手填路径
const browseBtn = document.querySelector("#edit-binary-browse");
globalThis.haveNativePick = false;
(async () => {
  try {
    const caps = await (await fetch("/api/capabilities")).json();
    globalThis.haveNativePick = !!caps.native_pick_available;
  } catch {}
  browseBtn.hidden = !globalThis.haveNativePick;
})();
browseBtn.onclick = async () => {
  try {
    const r = await invoke("pick_file");
    if (r.supported && r.path) document.querySelector("#edit-binary").value = r.path;
  } catch (e) {
    showNotice(String(e), true);
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
document.querySelector("#github-btn").onclick = () => window.open("https://github.com/Jonnyan404/universal-shell", "_blank");

// 设置模态
document.querySelector("#settings-btn").onclick = openSettings;
document.querySelector("#settings-modal-close").onclick = () => { settingsModal.hidden = true; };
document.querySelector("#sett-cancel").onclick = () => { settingsModal.hidden = true; };
document.querySelector("#settings-form").onsubmit = (e) => {
  e.preventDefault();
  saveSettings();
};
document.querySelector("#sett-check-update").onclick = () => checkShellUpdate(true);
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
  // 主题默认跟随系统
  const dark = matchMedia("(prefers-color-scheme: dark)").matches;
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  document.querySelector("#theme-btn").textContent = dark ? "☾" : "☀";

  programs = await invoke("get_programs");
  statuses = (await invoke("batch_status_local")) || [];
  renderSidebar();
  if (programs.length) {
    await switchCurrent(programs[0].id);
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

  // F-3/F10：运行状态改由 WS 事件推送；此处仅作 WS 断线兜底
  setInterval(refreshAllStatuses, 15000);
  setInterval(() => {
    if (current) refreshManageLog();
  }, 3000);
}

boot().catch((e) => {
  showNotice(String(e), true);
});