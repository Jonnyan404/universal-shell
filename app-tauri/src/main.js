const { invoke } = window.__TAURI__.core;
const { open, save, confirm } = window.__TAURI__.dialog;

let programs = [];
let current = null;
let values = {};
let statuses = [];

const el = {
  tabs: document.querySelector("#program-tabs"),
  progTitle: document.querySelector("#prog-title"),
  progSub: document.querySelector("#prog-sub"),
  chips: document.querySelector("#status-chips"),
  form: document.querySelector("#field-form"),
  actions: document.querySelector("#actions"),
  manageView: document.querySelector("#manage-view"),
  batchView: document.querySelector("#batch-view"),
  libraryView: document.querySelector("#library-view"),
  batchBody: document.querySelector("#batch-body"),
  registryBar: document.querySelector("#registry-bar"),
  libStatus: document.querySelector("#lib-status"),
  libSearch: document.querySelector("#lib-search"),
  libList: document.querySelector("#lib-list"),
  libSourceBar: document.querySelector("#lib-source-bar"),
  libFetchInfo: document.querySelector("#lib-fetch-info"),
  libCacheToggle: document.querySelector("#lib-cache-toggle"),
  libCacheDrawer: document.querySelector("#lib-cache-drawer"),
  libPager: document.querySelector("#lib-pager"),
};

let view = "manage";
let registries = [];
let registryUrl = "";
let manifest = null;
let libSearchValue = "";
let libLocalDirty = false;

// ---------- 国际化 ----------
const i18n = {
  dict: {},
  effective: "zh-CN",
  manual: "auto",
};

async function loadLocale() {
  const loc = await invoke("get_locale");
  i18n.effective = loc.effective || "zh-CN";
  i18n.manual = loc.manual || "auto";
  try {
    const resp = await fetch(`./locales/${i18n.effective}.json`);
    i18n.dict = (await resp.json()) || {};
  } catch {
    i18n.dict = {};
  }
  return loc;
}

// 取当前语言文案；key 缺失时原样返回 key（便于发现漏翻）。
function t(key, vars) {
  let s = i18n.dict[key];
  if (s === undefined) s = key;
  if (vars) {
    for (const [k, v] of Object.entries(vars)) {
      s = s.split(`%{${k}}`).join(String(v));
    }
  }
  return s;
}

// 处理静态元素上的 data-i18n / data-i18n-title / data-i18n-placeholder
function applyStaticI18n() {
  document.querySelectorAll("[data-i18n]").forEach((n) => {
    n.textContent = t(n.dataset.i18n);
  });
  document.querySelectorAll("[data-i18n-title]").forEach((n) => {
    n.setAttribute("title", t(n.dataset.i18nTitle));
  });
  document.querySelectorAll("[data-i18n-placeholder]").forEach((n) => {
    n.setAttribute("placeholder", t(n.dataset.i18nPlaceholder));
  });
  document.querySelector("html").setAttribute(
    "lang",
    i18n.effective === "en" ? "en" : "zh-CN"
  );
}

// 切换语言：写后端 → 重载字典 → 全量重渲染
async function changeLocale(locale) {
  await invoke("set_locale", { locale });
  await loadLocale();
  applyStaticI18n();
  const sw = document.querySelector("#lang-switcher");
  if (sw) {
    sw.querySelectorAll("option").forEach((o) => {
      o.selected = o.value === i18n.manual;
    });
  }
  await refreshAll();
  showNotice(t("toast.locale.applied"));
}

async function refreshAll() {
  await refresh();
  if (view === "batch") await refreshBatchLocal();
  else if (view === "library") renderLibrary();
}

// 全局 Toast 通知：右上角浮层，成功/失败着色，几秒后自动消失
function showNotice(text, isError, duration = 3200) {
  const container = document.querySelector("#toast-container");
  if (!container) return;
  const toast = document.createElement("div");
  toast.className = "toast" + (isError ? " error" : " ok");
  toast.textContent = text;
  container.appendChild(toast);
  requestAnimationFrame(() => toast.classList.add("show"));
  setTimeout(() => {
    toast.classList.remove("show");
    toast.addEventListener("transitionend", () => toast.remove(), { once: true });
    setTimeout(() => toast.remove(), 600);
  }, duration);
}

// ---------- 程序管理 ----------

async function refresh() {
  programs = await invoke("get_programs");
  const ml = document.querySelector("#manage-log");
  if (!programs.length) {
    el.progTitle.textContent = t("ui.not_imported");
    el.progSub.textContent = "";
    if (ml) ml.hidden = true;
    renderSidebar();
    return;
  }
  if (!current || !programs.some((p) => p.id === current.id)) {
    current = programs.find((p) => !p.hidden) || programs[0];
  }
  renderSidebar();
  await switchTo(current.id);
}

function renderSidebar() {
  el.tabs.innerHTML = "";
  // 侧栏只展示未隐藏的程序；隐藏程序在批量管理里可见
  const visible = programs.filter((p) => !p.hidden);
  if (!visible.length) {
    const hint = document.createElement("div");
    hint.style.cssText =
      "font-size:12px;color:var(--sidebar-sub);padding:12px 6px;";
    hint.textContent = t("side.empty_hint") + t("side.empty_suffix");
    el.tabs.appendChild(hint);
    return;
  }
  const stById = new Map(statuses.map((s) => [s.id, s.status]));
  for (const p of visible) {
    const item = document.createElement("div");
    item.className = p?.id === current?.id ? "prog-item active" : "prog-item";

    const st = stById.get(p.id);
    const on = st?.running ?? false;
    const dot = document.createElement("span");
    dot.className = "status-dot" + (on ? " on" : "");
    dot.title = on ? t("st.running") : t("st.stopped");

    const ico = document.createElement("span");
    ico.className = "prog-ico";
    ico.textContent = (p.name.trim().charAt(0) || "?").toUpperCase();

    const info = document.createElement("div");
    info.className = "info";
    const name = document.createElement("div");
    name.className = "name";
    const nameText = document.createElement("span");
    nameText.className = "nm";
    nameText.textContent = p.name;
    name.appendChild(nameText);
    const editIco = document.createElement("span");
    editIco.className = "prog-edit-ico";
    editIco.title = t("act.edit");
    editIco.textContent = "✎";
    editIco.onclick = (e) => {
      e.stopPropagation();
      openEditModal(p.id);
    };
    name.appendChild(editIco);
    const sub = document.createElement("div");
    sub.className = "sub";
    sub.textContent = !st
      ? p.repo || "—"
      : !st.installed
        ? t("st.not_installed", { repo: p.repo })
        : st.running
          ? t("st.running_ver", { ver: st.local_version })
          : t("st.stopped_ver", { ver: st.local_version });
    info.append(name, sub);

    item.append(dot, ico, info);
    item.onclick = () => switchToManage(p.id);
    item.title = `${p.name} · ${p.repo}`;
    el.tabs.appendChild(item);
  }
}

async function switchToManage(id) {
  if (view !== "manage") switchView("manage");
  await switchTo(id);
}

async function switchTo(id) {
  current = programs.find((p) => p.id === id);
  if (!current) return;
  values = await invoke("get_values", { programId: id });
  opLogs = [];
  renderForm();
  showManageLog();
  // 仅以本地状态渲染（不联网查版本）；版本更新只走手动「检查更新」
  await refreshStatusLocal();
  refreshManageLog();
}

// ---------- 下载 / 更新（状态栏与批量管理共用，防重复触发）----------

const installing = new Set();

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
  // install 命令已改为后台线程执行并立即返回；真实进度/完成/失败见 download-progress 事件
  try {
    await invoke("install", { programId: id });
  } catch (e) {
    installing.delete(id);
    if (btn) {
      btn.disabled = false;
      clearButtonProgress(btn);
      btn.textContent = btn.dataset.installed === "1" ? t("dl.update") : t("dl.download");
    }
    syncInstallBtns(id);
    showNotice(String(e), true);
    if (id === current?.id) logOp(t("toast.download_fail", { err: e }));
  }
}

function findDlButton(id) {
  if (current?.id === id) {
    const b = document.querySelector("#dl-btn");
    if (b) return b;
  }
  return document.querySelector(`#batch-body button[data-program-id="${id}"]`);
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

function handleDownloadProgress(payload) {
  const { program_id, stage, received, total, done, error, version } = payload || {};
  const btn = findDlButton(program_id);
  if (stage === "downloading") {
    const pct =
      total ? Math.min(99, Math.round((received / total) * 100)) : null;
    setButtonProgress(
      btn,
      pct == null ? 0 : pct,
      pct == null ? t("dl.downloading") : t("dl.progress", { pct }),
    );
  } else if (stage === "verifying" || stage === "extracting") {
    setButtonProgress(btn, 100, t("dl.working"));
  } else if (done) {
    completeInstall(program_id, null, version || "");
  } else if (error) {
    completeInstall(program_id, error, "");
  }
}

async function completeInstall(id, error, version) {
  installing.delete(id);
  try {
    if (view === "batch") await refreshBatchLocal();
    else if (current?.id === id) await refreshStatusLocal();
    else await refresh();
  } catch {}
  syncInstallBtns(id);
  const btn = findDlButton(id);
  clearButtonProgress(btn);
  if (btn) {
    btn.disabled = false;
    btn.textContent = btn.dataset.installed === "1" ? t("dl.update") : t("dl.download");
  }
  if (error) {
    showNotice(String(error), true);
    if (id === current?.id) logOp(t("toast.download_fail", { err: error }));
  } else {
    const p = programs.find((x) => x.id === id);
    showNotice(t("toast.updated", { name: p ? p.name : id, ver: version }));
    if (id === current?.id) logOp(t("toast.updated_short", { ver: version }));
  }
}

function syncInstallBtns(id) {
  const dlBtn = document.querySelector("#dl-btn");
  if (!dlBtn || current?.id !== id) return;
  const st = statuses.find((s) => s.id === id)?.status;
  const busy = installing.has(id);
  if (!st?.installed) {
    dlBtn.hidden = false;
    dlBtn.disabled = busy;
    dlBtn.textContent = busy ? t("dl.downloading") : t("dl.download");
  } else if (st.up_to_date || !st.latest_version) {
    // 已最新，或最新版本未知：不显示“更新”
    dlBtn.hidden = true;
  } else {
    dlBtn.hidden = false;
    dlBtn.disabled = busy;
    dlBtn.textContent = busy ? t("dl.downloading") : t("dl.update");
  }
}

async function renderForm() {
  el.progTitle.textContent = current.name;
  el.progSub.textContent = current.repo || "";
  el.form.innerHTML = "";
  for (const f of current.fields) {
    // 开机启动统一由批量管理页管理，程序页不再显示该字段
    if (f.kind === "autostart") continue;
    const row = document.createElement("div");
    row.className = "field-row";
    const label = document.createElement("label");
    label.textContent = f.required ? f.label + " *" : f.label;
    label.className = "field-label" + (f.required ? " required" : "");
    row.appendChild(label);

    if (f.kind === "boolean" || f.kind === "autostart") {
      const wrap = document.createElement("div");
      const check = document.createElement("input");
      check.type = "checkbox";
      check.checked = (values[f.key] ?? f.default) === "true";
      check.addEventListener("change", async () => {
        values[f.key] = check.checked ? "true" : "false";
        if (f.kind === "autostart") {
          try {
            await invoke("set_autostart", {
              programId: current.id,
              enabled: check.checked,
            });
            showNotice(t("toast.autostart_set"));
            logOp(check.checked ? t("toast.autostart_enabled") : t("toast.autostart_disabled"));
          } catch (e) {
            showNotice(String(e), true);
          }
        }
      });
      wrap.appendChild(check);
      row.appendChild(wrap);
    } else {
      const input = document.createElement("input");
      input.type = "text";
      input.value = values[f.key] ?? f.default;
      input.addEventListener("input", () => {
        values[f.key] = input.value;
      });
      row.appendChild(input);
      if (f.kind === "file" || f.kind === "directory") {
        const btn = document.createElement("button");
        btn.textContent = t("act.browse");
        btn.onclick = async () => {
          const picked = await open({
            multiple: false,
            directory: f.kind === "directory",
          });
          if (picked) {
            const path = Array.isArray(picked) ? picked[0] : picked;
            input.value = path;
            values[f.key] = path;
          }
        };
        row.appendChild(btn);
      }
    }
    el.form.appendChild(row);
  }

  el.actions.innerHTML = "";
  const start = document.createElement("button");
  start.id = "start-btn";
  start.textContent = t("act.start");
  start.onclick = async () => {
    try {
      const st = await invoke("start_program", {
        payload: { program_id: current.id, values },
      });
      renderStatus(st);
      await refreshStatusLocal();
      logOp(t("toast.started"));
      refreshManageLog();
    } catch (e) {
      showNotice(String(e), true);
      logOp(t("toast.start_fail", { err: e }));
    }
  };
  el.actions.appendChild(start);

  const stop = document.createElement("button");
  stop.id = "stop-btn";
  stop.disabled = true;
  stop.textContent = t("act.stop");
  stop.onclick = async () => {
    try {
      const st = await invoke("stop_program", { programId: current.id });
      renderStatus(st);
      await refreshStatusLocal();
      logOp(t("toast.stopped"));
      refreshManageLog();
    } catch (e) {
      showNotice(String(e), true);
      logOp(t("toast.stop_fail", { err: e }));
    }
  };
  el.actions.appendChild(stop);

  const restart = document.createElement("button");
  restart.id = "restart-btn";
  restart.disabled = true;
  restart.textContent = t("act.restart");
  restart.onclick = async () => {
    try {
      const st = await invoke("restart_program", {
        payload: { program_id: current.id, values },
      });
      renderStatus(st);
      await refreshStatusLocal();
      logOp(t("toast.restarted"));
      refreshManageLog();
    } catch (e) {
      showNotice(String(e), true);
      logOp(t("toast.restart_fail", { err: e }));
    }
  };
  el.actions.appendChild(restart);

  const appDir = document.createElement("button");
  appDir.textContent = t("act.open_app_dir");
  appDir.title = t("act.open_app_dir");
  appDir.onclick = () => revealAppDir(current.id);
  el.actions.appendChild(appDir);

  // 有地址(/端口)字段时，提供「打开网站」「复制地址」
  if (webUrl(current.id)) {
    const open = document.createElement("button");
    open.className = "icon-btn";
    open.title = t("act.open_site");
    open.textContent = "↗";
    open.onclick = () => {
      const u = webUrl(current.id);
      if (u) openWeb(u);
    };
    el.actions.appendChild(open);

    const copy = document.createElement("button");
    copy.className = "icon-btn";
    copy.title = t("act.copy_addr");
    copy.textContent = "⧉";
    copy.onclick = () => {
      const u = webUrl(current.id);
      if (u) copyLogText(u);
    };
    el.actions.appendChild(copy);
  }
}

// 构造程序的 Web 访问地址(如有地址/端口字段)。无地址返回 null
function webUrl(id) {
  const prog = programs.find((p) => p.id === id);
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

// 用系统默认浏览器打开地址
async function openWeb(url) {
  try {
    await window.__TAURI__.opener.openUrl(url);
  } catch (e) {
    showNotice(String(e), true);
  }
}

// 仅本地状态（无网络）：程序自行退出/报错后，刷新启动按钮与运行态
async function refreshStatusLocal() {
  if (!current || view !== "manage") return;
  try {
    const st = await invoke("get_status_local", { programId: current.id });
    renderStatus(st);
  } catch {
    // 忽略
  }
}

function renderStatus(st) {
  // 回写本地缓存，保证侧栏红绿圆点/批量视图即时一致
  if (current) {
    const idx = statuses.findIndex((s) => s.id === current.id);
    if (idx >= 0) {
      statuses[idx].status = st;
    } else {
      statuses.push({ id: current.id, name: current.name, status: st });
    }
  }
  const b = document.querySelector("#start-btn");
  const stop = document.querySelector("#stop-btn");
  const restart = document.querySelector("#restart-btn");
  const chips = [
    { text: t("st.local_ver", { ver: st.local_version }), cls: "" },
    { text: st.installed ? t("st.installed") : t("st.not_installed_bare"), cls: "" },
  ].filter((c) => c.text !== "");
  if (st.autostart) {
    chips.push({ text: t("st.autostart"), cls: "ok" });
  }
  el.chips.innerHTML = "";
  for (const c of chips) {
    const span = document.createElement("span");
    span.textContent = c.text;
    span.className = "pill" + (c.cls ? " " + c.cls : "");
    el.chips.appendChild(span);
  }
  if (b) b.disabled = !!st.running;
  if (stop) stop.disabled = !st.running;
  if (restart) restart.disabled = !st.running;
  const dlBtn = document.querySelector("#dl-btn");
  if (dlBtn) {
    dlBtn.dataset.installed = st.installed ? "1" : "0";
    if (st.installed && st.up_to_date) {
      // 已是最新版本：隐藏更新按钮
      dlBtn.hidden = true;
    } else if (st.installed && !st.latest_version) {
      // 已安装但最新版本未知（未联网刷新）：不擅自显示“更新”，避免误导
      dlBtn.hidden = true;
    } else {
      dlBtn.hidden = false;
      const busy = installing.has(current.id);
      dlBtn.disabled = busy;
      dlBtn.textContent = busy ? t("dl.downloading") : st.installed ? t("dl.update") : t("dl.download");
    }
  }
  renderSidebar();
}

async function revealLogs(id) {
  try {
    await invoke("reveal_logs", { programId: id });
  } catch (e) {
    showNotice(String(e), true);
  }
}

async function revealAppDir(id) {
  try {
    await invoke("reveal_app_dir", { programId: id });
  } catch (e) {
    showNotice(String(e), true);
  }
}

// ---------- 批量管理 ----------

// 仅本地状态（无网络）：定时器与“刷新状态”用它，避免自动联网
async function refreshBatchLocal() {
  try {
    statuses = await invoke("batch_status_local");
    renderSidebar();
    renderBatch();
  } catch (e) {
    showNotice(String(e), true);
  }
}

// 手动“检查更新”：完整联网比对最新版本
async function checkUpdates() {
  const btn = document.querySelector("#batch-check-updates");
  const refreshing = btn?.classList.contains("busy");
  if (refreshing) return;
  if (btn) {
    btn.classList.add("busy");
    btn.disabled = true;
    btn.textContent = t("dl.checking_short");
  }
  showNotice(t("dl.checking"));
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
  // “上次检查更新”全局提示：取各程序最近一次联网检查的最大时间戳
  const checkedAt = statuses.reduce(
    (m, it) => {
      const t = it.status?.latest_checked_at;
      return t && t > m ? t : m;
    },
    0
  );
  const updLine = document.querySelector("#batch-checked-at");
  if (updLine) {
    updLine.textContent = checkedAt
      ? t("check.last_checked", { ago: timeAgo(checkedAt) })
      : t("check.not_checked");
    updLine.title = checkedAt
      ? t("check.last_online", { date: fmtDate(checkedAt) })
      : t("lib.latest_checked_hint");
  }
  if (!statuses.length) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 7;
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
        await invoke("set_autostart", {
          programId: item.id,
          enabled: auto.checked,
        });
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
        await invoke("set_program_hidden", {
          programId: item.id,
          hidden: hide.checked,
        });
        showNotice(
          hide.checked
            ? t("toast.hidden", { name: item.name })
            : t("toast.unhidden", { name: item.name })
        );
        if (item.id === current?.id && hide.checked) current = null;
        programs = await invoke("get_programs");
        if (!current) current = programs.find((p) => !p.hidden) || null;
        await refreshBatchLocal();
        if (current) await switchTo(current.id);
        else await refresh();
      } catch (e) {
        hide.checked = !hide.checked;
        showNotice(String(e), true);
      } finally {
        hide.disabled = false;
      }
    });
    tdHide.appendChild(hide);
    tr.appendChild(tdHide);

    const tdOps = document.createElement("td");
    const ops = document.createElement("span");
    ops.className = "batch-ops";

    // 下载 / 更新（复用状态栏安装逻辑，防重复触发；已最新时置灰）
    const isUpToDate = s.installed && s.up_to_date;
    const dl = makeOpBtn(s.installed ? (isUpToDate ? t("st.latest") : t("dl.update")) : t("dl.download"), async () => {
      dl.dataset.programId = item.id;
      dl.classList.add("dl-progress-btn");
      dl.dataset.installed = s.installed ? "1" : "0";
      // 最新版本未知时先联网检查，避免不必要的覆盖安装
      if (s.installed && !s.latest_version) {
        dl.disabled = true;
        dl.textContent = t("dl.checking_short");
        try {
          const full = await invoke("batch_status");
          statuses = full;
          const fresh = statuses.find((x) => x.id === item.id);
          renderSidebar();
          if (fresh?.status?.up_to_date) {
            renderBatch();
            showNotice(t("st.is_latest", { name: item.name }));
            return;
          }
        } catch (e) {
          renderBatch();
          showNotice(String(e), true);
          return;
        }
      }
      installProgram(item.id, dl);
    });
    if (isUpToDate) dl.disabled = true;
    ops.appendChild(dl);

    const start = makeOpBtn(t("act.start"), async () => {
      try {
        await invoke("start_program", {
          payload: { program_id: item.id, values: {} },
        });
        await refreshBatchLocal();
      } catch (e) {
        showNotice(String(e), true);
      }
    });
    ops.appendChild(start);

    const restart = makeOpBtn(t("act.restart"), async () => {
      try {
        await invoke("restart_program", {
          payload: { program_id: item.id, values: {} },
        });
        await refreshBatchLocal();
      } catch (e) {
        showNotice(String(e), true);
      }
    });
    ops.appendChild(restart);

    const stop = makeOpBtn(t("act.stop"), async () => {
      try {
        await invoke("stop_program", { programId: item.id });
        await refreshBatchLocal();
      } catch (e) {
        showNotice(String(e), true);
      }
    });
    ops.appendChild(stop);

    const logs = makeOpBtn(t("act.log"), () => openLogModal(item.id));
    ops.appendChild(logs);

    const openDir = makeOpBtn(t("act.open_app_dir"), () => revealAppDir(item.id));
    ops.appendChild(openDir);

    const edit = makeOpBtn(t("act.edit"), () => openEditModal(item.id));
    ops.appendChild(edit);

    const del = makeOpBtn(t("act.delete"), () => confirmAndDelete(item.id, item.name));
    del.className = "op-btn op-danger";
    ops.appendChild(del);

    tdOps.appendChild(ops);
    tr.appendChild(tdOps);
    el.batchBody.appendChild(tr);
  }
}

function makeOpBtn(label, fn) {
  const b = document.createElement("button");
  b.textContent = label;
  b.className = "op-btn";
  b.onclick = fn;
  return b;
}

// ---------- 网络 / 代理设置 ----------

// 把已保存的代理 URL 拆分为 类型/地址/用户名/密码（兼容无 scheme 的旧格式）
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
    } else {
      out.user = decodeURIComponent(cred);
    }
  }
  out.host = s;
  if (out.type === "socks5h" || out.type === "socks") out.type = "socks5";
  if (out.type === "https") out.type = "http";
  return out;
}

// 把 类型/地址/用户名/密码 拼回代理 URL 字符串（用户名密码经 URL 编码）
function buildProxy(type, host, user, pass) {
  if (!host) return "";
  const cred =
    user || pass ? `${encodeURIComponent(user)}:${encodeURIComponent(pass)}@` : "";
  return `${type}://${cred}${host}`;
}

function openSettings() {
  const modal = document.querySelector("#settings-modal");
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
    .then((on) => {
      document.querySelector("#sett-shell-auto").checked = !!on;
    })
    .catch(() => {});
  modal.hidden = false;
}

function closeSettings() {
  document.querySelector("#settings-modal").hidden = true;
}

async function saveSettings() {
  const acc = document.querySelector("#sett-accelerate").value;
  const type = document.querySelector("#sett-proxy-type").value;
  const host = document.querySelector("#sett-proxy-host").value.trim();
  const user = document.querySelector("#sett-proxy-user").value.trim();
  const pass = document.querySelector("#sett-proxy-pass").value;
  const hp = buildProxy(type, host, user, pass);
  const shellAuto = document.querySelector("#sett-shell-auto").checked;
  try {
    await invoke("set_proxy", { acceleratePrefix: acc, httpProxy: hp });
    try {
      await invoke("set_shell_autostart", { enabled: shellAuto });
    } catch (e) {
      showNotice(t("toast.shell_autostart_fail", { err: e }), true);
    }
    showNotice(t("toast.settings_saved"));
  } catch (e) {
    showNotice(String(e), true);
  }
  closeSettings();
}

// ---------- 日志查看（合并 stdout/stderr 单窗口，stderr 红色）----------

let logProgramId = null;

// 把合并日志文本渲染进容器：\x1F 开头的行视为 stderr，着红色
function renderLogBody(container, text) {
  container.innerHTML = "";
  if (!text) {
    container.textContent = t("st.empty");
    return;
  }
  const lines = text.split("\n");
  const frag = document.createDocumentFragment();
  for (const line of lines) {
    const isErr = line.charCodeAt(0) === 0x1f;
    const content = isErr ? line.slice(1) : line;
    const span = document.createElement("span");
    span.textContent = content;
    if (isErr) span.classList.add("log-err");
    frag.appendChild(span);
    frag.appendChild(document.createTextNode("\n"));
  }
  container.appendChild(frag);
}

function openLogModal(id) {
  logProgramId = id;
  document.querySelector("#log-modal").hidden = false;
  refreshLog();
}

async function refreshLog() {
  const content = document.querySelector("#log-content");
  const title = document.querySelector("#log-modal-title");
  const p = programs.find((x) => x.id === logProgramId);
  title.textContent = t("log.title_fmt", { name: p ? p.name : logProgramId });
  try {
    const logs = await invoke("get_logs", { programId: logProgramId });
    renderLogBody(content, logs.text);
  } catch (e) {
    content.textContent = String(e);
  }
}

function closeLogModal() {
  document.querySelector("#log-modal").hidden = true;
  logProgramId = null;
}

// 复制文本到剪贴板（带旧 API 兜底）
async function copyLogText(text) {
  try {
    await navigator.clipboard.writeText(text);
    showNotice(t("toast.copied"));
    return;
  } catch {
    // 回退：隐藏 textarea + execCommand
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    try {
      document.execCommand("copy");
      showNotice(t("toast.copied"));
    } catch {
      showNotice(t("toast.copy_fail"), true);
    }
    ta.remove();
  }
}

// ---------- 程序管理内嵌日志 ----------

// 会话内操作日志（启动/停止/下载/更新等），显示在日志窗口顶部，便于回看操作轨迹
let opLogs = [];

function logOp(msg) {
  const t = new Date().toLocaleTimeString();
  opLogs.push(`[${t}] ${msg}`);
  if (opLogs.length > 200) opLogs.shift();
  if (view === "manage" && current) renderOpLog();
  // 同步落盘到壳操作日志（由后端统一加时间戳），无权限时静默忽略
  invoke("log_shell_op", { msg }).catch(() => {});
}

function showManageLog() {
  const box = document.querySelector("#manage-log");
  const ope = document.querySelector("#op-log");
  if (box) box.hidden = !current || view !== "manage";
  if (ope) ope.hidden = !current || view !== "manage";
  if (!current || view !== "manage") return;
  renderOpLog();
  refreshManageLog();
}

// 渲染操作日志（启动/停止/下载/更新轨迹），独立条带显示在日志上方
function renderOpLog() {
  const ope = document.querySelector("#op-log");
  if (!ope || ope.hidden) return;
  if (!opLogs.length) {
    ope.innerHTML = "";
    ope.hidden = true;
    return;
  }
  const frag = document.createDocumentFragment();
  for (const o of opLogs) {
    const span = document.createElement("span");
    span.textContent = o;
    frag.appendChild(span);
  }
  ope.innerHTML = "";
  ope.appendChild(frag);
  ope.scrollTop = ope.scrollHeight;
}

async function refreshManageLog() {
  if (!current || view !== "manage") return;
  const box = document.querySelector("#manage-log");
  if (!box || box.hidden) return;
  const content = document.querySelector("#manage-log-content");
  const nearBottom =
    content.scrollTop + content.clientHeight >= content.scrollHeight - 48;
  try {
    const logs = await invoke("get_logs", { programId: current.id });
    renderLogBody(content, logs.text);
    if (nearBottom) content.scrollTop = content.scrollHeight;
  } catch (e) {
    content.textContent = String(e);
  }
}

// 上下拖动调节内嵌日志窗口高度（最小 80px，不超过窗口的 70%）
function setupManageLogResize() {
  const log = document.querySelector("#manage-log");
  const handle = document.querySelector("#manage-log-resize");
  if (!log || !handle) return;
  let dragging = false;
  let startY = 0;
  let startH = 0;

  const onMove = (e) => {
    if (!dragging || log.classList.contains("fullscreen")) return;
    const dh = e.clientY - startY;
    const maxH = window.innerHeight * 0.7;
    const h = Math.min(Math.max(startH - dh, 80), maxH);
    log.style.height = h + "px";
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

// ---------- 删除 ----------

async function confirmAndDelete(id, name) {
  const ok = window.confirm(
    t("confirm.delete", { name })
  );
  if (!ok) return;
  try {
    await invoke("delete_program", { programId: id });
    showNotice(t("toast.deleted", { name }));
    if (current?.id === id) logOp(t("toast.deleted", { name }));
    programs = await invoke("get_programs");
    if (current?.id === id) {
      current = null;
      await refresh();
    } else {
      renderSidebar();
    }
    if (view === "batch") await refreshBatchLocal();
  } catch (e) {
    showNotice(String(e), true);
  }
}

// ---------- 编辑 ----------

let editProgramId = null;

function openEditModal(id) {
  const p = programs.find((x) => x.id === id);
  if (!p) return;
  editProgramId = id;
  document.querySelector("#edit-name").value = p.name;
  document.querySelector("#edit-desc").value = p.description || "";
  document.querySelector("#edit-repo").value = p.repo;
  document.querySelector("#edit-binary").value = p.binary;
  document.querySelector("#edit-args").value = (p.args || []).join(" ");
  renderEditFields(p.fields || []);
  document.querySelector("#edit-modal").hidden = false;
}

function renderEditFields(fields) {
  const body = document.querySelector("#edit-fields-body");
  body.innerHTML = "";
  for (const f of fields) {
    body.appendChild(editFieldRow(f));
  }
}

function editFieldRow(f) {
  const row = document.createElement("div");
  row.className = "edit-field-row";
  const k = document.createElement("input");
  k.className = "k";
  k.placeholder = t("lib.field_key");
  k.value = f.key;
  const l = document.createElement("input");
  l.className = "l";
  l.placeholder = t("lib.labels");
  l.value = f.label || f.key;
  const sel = document.createElement("select");
  sel.className = "kind";
  const kinds = ["string", "boolean", "file", "directory", "autostart"];
  for (const kd of kinds) {
    const opt = document.createElement("option");
    opt.value = kd;
    opt.textContent = kd;
    if (kd === f.kind) opt.selected = true;
    sel.appendChild(opt);
  }
  const d = document.createElement("input");
  d.className = "d";
  d.placeholder = t("lib.def_val");
  d.value = f.default ?? "";
  const reqWrap = document.createElement("label");
  reqWrap.className = "req";
  reqWrap.title = t("lib.precheck");
  const req = document.createElement("input");
  req.type = "checkbox";
  req.checked = !!f.required;
  const reqTxt = document.createElement("span");
  reqTxt.textContent = t("lib.required");
  reqWrap.append(req, reqTxt);
  const reqMark = document.createElement("span");
  reqMark.className = "req-star";
  reqMark.textContent = "*";
  reqMark.hidden = !f.required;
  const syncReq = () => {
    reqMark.hidden = !req.checked;
    row.classList.toggle("req-on", req.checked);
  };
  req.addEventListener("change", syncReq);
  syncReq();
  const del = document.createElement("button");
  del.className = "edit-field-del";
  del.type = "button";
  del.textContent = "✕";
  del.title = t("act.delete_field");
  del.onclick = () => row.remove();
  row.append(k, l, sel, d, reqWrap, reqMark, del);
  return row;
}

async function saveEdit() {
  const fields = [];
  document
    .querySelectorAll("#edit-fields-body .edit-field-row")
    .forEach((row) => {
      const k = row.querySelector(".k").value.trim();
      const l = row.querySelector(".l").value.trim();
      const kind = row.querySelector(".kind").value;
      const def = row.querySelector(".d").value;
      const required = row.querySelector(".req input").checked;
      if (!k) return;
      fields.push({ key: k, kind, label: l || k, default: def, required });
    });
  const payload = {
    id: editProgramId,
    name: document.querySelector("#edit-name").value.trim(),
    description: document.querySelector("#edit-desc").value.trim(),
    repo: document.querySelector("#edit-repo").value.trim(),
    binary: document.querySelector("#edit-binary").value.trim(),
    args: document
      .querySelector("#edit-args")
      .value.trim()
      .split(/\s+/)
      .filter(Boolean),
    fields,
  };
  try {
    await invoke("edit_program", { payload });
    showNotice(t("toast.saved"));
    logOp(t("toast.settings_saved_alt"));
    const editedId = editProgramId;
    closeEditModal();
    programs = await invoke("get_programs");
    current = programs.find((p) => p.id === editedId) || current;
    if (current) {
      await switchTo(current.id);
    }
    if (view === "batch") await refreshBatchLocal();
    renderSidebar();
  } catch (e) {
    showNotice(String(e), true);
  }
}

function closeEditModal() {
  document.querySelector("#edit-modal").hidden = true;
  editProgramId = null;
}

// ---------- 视图切换 ----------

function switchView(v) {
  view = v;
  el.manageView.hidden = v !== "manage";
  el.batchView.hidden = v !== "batch";
  el.libraryView.hidden = v !== "library";
  const dlBtn = document.querySelector("#dl-btn");
  if (dlBtn) dlBtn.hidden = v !== "manage";
  if (v === "manage") {
    if (current) switchTo(current.id);
    showManageLog();
  } else {
    const ml = document.querySelector("#manage-log");
    if (ml) ml.hidden = true;
    const ope = document.querySelector("#op-log");
    if (ope) ope.hidden = true;
    el.progTitle.textContent = v === "batch" ? t("batch.title") : t("lib.title");
    el.progSub.textContent =
      v === "batch" ? t("check.title") : t("lib.select_local");
    el.chips.innerHTML = "";
    if (v === "batch") refreshBatchLocal();
    else {
      if (!manifest) ensureLibraryFromCache();
      else renderLibrary();
    }
  }
}

// ---------- 模板库 ----------

// 模板库状态
let libSource = null; // 当前浏览的源 base；null=全部(合并)
let libPage = 0;
const LIB_PAGE_SIZE = 20;

function fmtDate(secs) {
  if (!secs) return "—";
  try {
    return new Date(secs * 1000).toLocaleString();
  } catch {
    return String(secs);
  }
}

function timeAgo(secs) {
  if (!secs) return t("st.never");
  const diff = (
    (Date.now() / 1000) - secs
  );
  if (diff < 60) return t("st.just");
  if (diff < 3600) return t("time.min_ago", { n: Math.floor(diff / 60) });
  if (diff < 86400) return t("time.hour_ago", { n: Math.floor(diff / 3600) });
  if (diff < 86400 * 30) return t("time.day_ago", { n: Math.floor(diff / 86400) });
  return fmtDate(secs);
}

// 进入模板库时若尚未加载清单，则用本地缓存恢复上一次刷新的远程源列表（不联网）。
async function ensureLibraryFromCache() {
  try {
    if (manifest) { renderLibrary(); return; }
    manifest = await invoke("get_merged_manifest_offline");
    libPage = 0;
  } catch (e) {
    // 无本地缓存：保持空态，renderLibrary 会提示先联网刷新
  }
  renderLibrary();
}

async function refreshLibrary() {
  try {
    const m = await invoke("get_merged_manifest", { registryUrl: registryUrl || "" });
    manifest = m;
    if (libSource && !(manifest.sources || []).some((s) => s[0] === libSource)) {
      libSource = null;
    }
    libPage = 0;
    renderLibrary();
  } catch (e) {
    el.libStatus.textContent = t("toast.manifest_fail", { err: e });
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
      : t("lib.summary", {
          n: manifest.sources.length,
          offline: nOffline ? t("lib.summary_offline", { n: nOffline }) : "",
          m: manifest.templates.length,
        });

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
  if (!slice.length) {
    el.libList.innerHTML = '<div class="lib-desc">' + t("lib.no_match") + "</div>";
  }
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
    repo.textContent = tpl.repo;

    const imported = programs.some((p) => p.id === id);
    const conflict = (manifest.conflicts || []).find(([cid]) => cid === id);
    if (conflict && conflict[1] > 1) {
      const mark = document.createElement("span");
      mark.className = "lib-cat lib-conflict";
      mark.textContent = t("lib.multi_source", { n: conflict[1] });
      top.append(mark);
    }

    const btn = document.createElement("button");
    // 已导入也可再次导入（doImport 内对已存在程序二次确认覆盖）
    btn.textContent = importing.has(id) ? t("lib.importing") : t("act.import");
    btn.disabled = importing.has(id);
    btn.onclick = () => doImport(id, base, btn);
    top.append(h, cat, repo, btn);

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
  for (const src of manifest?.sources || []) {
    const [base, offline, fetched] = src;
    const opt = document.createElement("option");
    opt.value = base;
    opt.textContent =
      (offline ? t("lib.offline") : "") + base + (fetched ? ` · ${fmtDate(fetched)}` : "");
    if (libSource === base) opt.selected = true;
    sel.appendChild(opt);
  }
  sel.onchange = () => {
    libSource = sel.value === "__all__" ? null : sel.value;
    libPage = 0;
    renderLibrary();
  };

  // 远程源更新：放在源栏下方一行，便于完整展示（悬停看精确时间）
  if (el.libFetchInfo) {
    const updateSpan = document.createElement("div");
    updateSpan.className = "lib-fetch-compact";
    const srcs = manifest?.sources || [];
    if (!manifest) {
      updateSpan.textContent = t("lib.remote_sources");
      updateSpan.title = t("lib.no_sources_hint");
    } else if (!srcs.length) {
      updateSpan.textContent = t("lib.remote_none");
      updateSpan.title = "";
    } else {
      const parts = [];
      const titles = [];
      for (const [base, offline, fetched] of srcs) {
        titles.push(
          base +
            (fetched
              ? t("lib.last_pull", { date: fmtDate(fetched) })
              : t("lib.not_pulled"))
        );
        parts.push(
          offline
            ? t("lib.offline_status", { suffix: fetched ? t("lib.cache") + timeAgo(fetched) : t("lib.no_cache") })
            : t("lib.online_status", { suffix: fetched ? t("log.equal_parts") + " " + timeAgo(fetched) : t("log.just") })
        );
      }
      updateSpan.textContent = t("log.remote_ready") + parts.join("　");
      updateSpan.title = titles.join("；");
    }
    el.libFetchInfo.innerHTML = "";
    el.libFetchInfo.appendChild(updateSpan);
  }

  const refresh = document.createElement("button");
  refresh.textContent = t("act.refresh");
  refresh.onclick = async () => {
    refresh.disabled = true;
    refresh.textContent = t("lib.pull");
    try {
      await refreshLibrary();
    } finally {
      refresh.disabled = false;
      refresh.textContent = t("act.refresh");
    }
  };
  el.libSourceBar.append(sel, refresh);
}

function renderPager(pages) {
  if (!el.libPager) return;
  el.libPager.innerHTML = "";
  if (pages <= 1) return;
  const prev = document.createElement("button");
  prev.textContent = t("lib.prev");
  prev.disabled = libPage <= 0;
  prev.onclick = () => {
    libPage--;
    renderLibrary();
  };
  const info = document.createElement("span");
  info.className = "lib-page-info";
  info.textContent = `${libPage + 1} / ${pages}`;
  const next = document.createElement("button");
  next.textContent = t("lib.next");
  next.disabled = libPage >= pages - 1;
  next.onclick = () => {
    libPage++;
    renderLibrary();
  };
  el.libPager.append(prev, info, next);
}

function renderLocalTemplates() {
  const list = programs || [];
  if (el.libCacheToggle) {
    el.libCacheToggle.textContent = t("lib.local") + `(${list.length})`;
  }
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
    const show = document.createElement("button");
    show.className = "op-btn";
    show.textContent = t("act.manage");
    show.onclick = () => switchTo(p.id);
    ops.append(show);
    row.append(name, repo, ops);
    el.libCacheDrawer.appendChild(row);
  }
}

async function doImport(id, base, btn) {
  if (importing.has(id)) return;
  importing.add(id);
  if (btn) {
    btn.disabled = true;
    btn.textContent = t("lib.importing");
  }
  try {
    const exists = programs.some((p) => p.id === id);
    let overwrite = false;
    if (exists) {
      const ok = await confirm(
        t("confirm.overwrite_import", { id }),
        { title: t("lib.overwrite"), kind: "warning" }
      );
      if (!ok) {
        importing.delete(id);
        renderLibrary();
        return;
      }
      overwrite = true;
    }
    await invoke("import_template", {
      registryUrl: base || registryUrl || "",
      templateId: id,
      overwrite,
    });
    showNotice(exists ? t("toast.import_overwritten", { id }) : t("toast.import_done", { id }));
    await afterImport(id);
    renderLibrary();
  } catch (e) {
    showNotice(String(e), true);
  } finally {
    importing.delete(id);
    if (btn) {
      btn.disabled = false;
      btn.textContent = t("act.import");
    }
  }
}

async function afterImport(id) {
  programs = await invoke("get_programs");
  if (!current || !programs.some((p) => p.id === current.id)) {
    current = programs[0];
  }
  renderSidebar();
  if (programs.some((p) => p.id === id)) await switchTo(id);
}

async function importLocalFile() {
  try {
    const file = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!file) return;
    let overwrite = false;
    try {
      await invoke("import_local_template", {
        templatePath: String(file),
        overwrite: false,
      });
    } catch (e) {
      if (/已存在|already exists/i.test(String(e))) {
        const ok = await confirm(
          t("lib.already_exists"),
          { title: t("lib.overwrite"), kind: "warning" }
        );
        if (!ok) return;
        overwrite = true;
        await invoke("import_local_template", {
          templatePath: String(file),
          overwrite: true,
        });
      } else {
        throw e;
      }
    }
    showNotice(t("lib.imported_local"));
    programs = await invoke("get_programs");
    renderSidebar();
  } catch (e) {
    showNotice(String(e), true);
  }
}

function registerInput(input, cb) {
  input.addEventListener("input", () => cb(input.value));
}

// ---------- 模板源管理 ----------

function openSourcesModal() {
  renderSourcesList();
  document.querySelector("#sources-modal").hidden = false;
}
function closeSourcesModal() {
  document.querySelector("#sources-modal").hidden = true;
}
function sourceRows() {
  return [...document.querySelectorAll("#sources-list .source-row")];
}
function renderSourcesList() {
  const box = document.querySelector("#sources-list");
  box.innerHTML = "";
  (registries.length ? registries : [""]).forEach((r, i) => {
    box.appendChild(sourceRowEl(r, i === 0));
  });
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
function addSourceRow() {
  const val = document.querySelector("#sources-new").value.trim();
  document
    .querySelector("#sources-list")
    .appendChild(sourceRowEl(val, false));
  document.querySelector("#sources-new").value = "";
}
async function saveSources() {
  const list = sourceRows()
    .map((r) => r.querySelector("input").value.trim())
    .filter(Boolean);
  if (!list.length) {
    showNotice(t("lib.keep_one"), true);
    return;
  }
  try {
    await invoke("set_registries", { registries: list });
    registries = await invoke("get_registries");
    registryUrl = registries[0] ?? "";
    libSource = registries[0] ?? null;
    showNotice(t("toast.sources_saved"));
    closeSourcesModal();
    manifest = null;
    renderLibrary();
  } catch (e) {
    showNotice(String(e), true);
  }
}

const importing = new Set();

// ---------- 侧栏收窄 / 主题 ----------

const THEME_MODES = ["", "light", "dark"];
function applyTheme() {
  const cur = document.documentElement.dataset.theme || "";
  const dark =
    cur === "dark" ||
    (!cur &&
      matchMedia("(prefers-color-scheme: dark)").matches);
  const b = document.querySelector("#theme-btn");
  if (b) {
    b.textContent = dark ? "☀" : "☾";
    if (cur) {
      b.title = dark ? t("ui.theme.light") : t("ui.theme.dark");
    } else {
      b.title = t("ui.theme.follow_system");
    }
  }
}
function cycleTheme() {
  const cur = document.documentElement.dataset.theme || "";
  const next = THEME_MODES[(THEME_MODES.indexOf(cur) + 1) % THEME_MODES.length];
  document.documentElement.dataset.theme = next;
  localStorage.setItem("theme", next);
  applyTheme();
}

const SIDEBAR_MODES = ["full", "narrow"];
function applySidebar(mode) {
  document.documentElement.dataset.sidebar = mode;
  const cb = document.querySelector("#collapse-btn");
  if (cb) {
    cb.textContent = mode === "full" ? "«" : "»";
    cb.title = mode === "full" ? t("ui.collapse") : t("ui.expand");
  }
}
function cycleSidebar() {
  const cur = localStorage.getItem("sidebarMode") || "full";
  const next =
    SIDEBAR_MODES[(SIDEBAR_MODES.indexOf(cur) + 1) % SIDEBAR_MODES.length];
  localStorage.setItem("sidebarMode", next);
  applySidebar(next);
}

// ---------- 启动 ----------

window.addEventListener("DOMContentLoaded", async () => {
  document.documentElement.dataset.theme =
    localStorage.getItem("theme") || "";
  applyTheme();
  applySidebar(localStorage.getItem("sidebarMode") || "full");
  await loadLocale();
  applyStaticI18n();
  applyTheme();
  // 后台安装进度事件：实时更新下载按钮进度条与完成/失败终态
  window.__TAURI__.event
    .listen("download-progress", (e) => handleDownloadProgress(e.payload))
    .catch(() => {});
  // 程序退出事件（由后端监视线程在探测到进程退出时广播）：即时刷新状态，恢复「启动」按钮
  window.__TAURI__.event
    .listen("process-exited", (e) => {
      const pid = e.payload?.program_id;
      if (view === "manage" && pid === current?.id) refreshStatusLocal();
    })
    .catch(() => {});
  const langBtn = document.querySelector("#lang-btn");
  if (langBtn) {
    const updateLangBtn = () => {
      const names = { auto: "文", "zh-CN": "中", en: "EN" };
      langBtn.textContent = names[i18n.manual] || "文";
      const hint = {
        auto: t("lang.auto"),
        "zh-CN": t("lang.zh"),
        en: t("lang.en"),
      };
      langBtn.title = hint[i18n.manual] || t("lang.title");
    };
    updateLangBtn();
    const seq = ["zh-CN", "en", "auto"];
    langBtn.onclick = async () => {
      const next = seq[(seq.indexOf(i18n.manual) + 1) % seq.length] || "auto";
      await changeLocale(next);
      updateLangBtn();
    };
  }
  const themeBtn = document.querySelector("#theme-btn");
  if (themeBtn) themeBtn.onclick = () => cycleTheme();
  const collapseBtn = document.querySelector("#collapse-btn");
  if (collapseBtn) collapseBtn.onclick = () => cycleSidebar();
  const batchLink = document.querySelector("#batch-link");
  if (batchLink) batchLink.onclick = () => switchView("batch");
  const libraryLink = document.querySelector("#library-link");
  if (libraryLink) libraryLink.onclick = () => switchView("library");
  const refreshBtn = document.querySelector("#batch-refresh");
  if (refreshBtn) refreshBtn.onclick = () => refreshBatchLocal();
  const checkUpdatesBtn = document.querySelector("#batch-check-updates");
  if (checkUpdatesBtn) checkUpdatesBtn.onclick = () => checkUpdates();
  const sa = document.querySelector("#batch-stop-all");
  if (sa)
    sa.onclick = async () => {
      try {
        await invoke("stop_all");
        showNotice(t("toast.stop_all"));
        await refreshBatchLocal();
      } catch (e) {
        showNotice(String(e), true);
      }
    };
  const dlBtn = document.querySelector("#dl-btn");
  if (dlBtn) dlBtn.onclick = () => installProgram(current?.id, dlBtn);

  // 网络设置模态
  const settingsBtn = document.querySelector("#settings-btn");
  if (settingsBtn) settingsBtn.onclick = openSettings;
  const settModal = document.querySelector("#settings-modal");
  document.querySelector("#settings-modal-close").onclick = closeSettings;
  document.querySelector("#sett-cancel").onclick = closeSettings;
  document.querySelector("#settings-form").onsubmit = (e) => {
    e.preventDefault();
    saveSettings();
  };
  settModal.addEventListener("click", (e) => {
    if (e.target === settModal) closeSettings();
  });

  // 日志模态
  const logModal = document.querySelector("#log-modal");
  document.querySelector("#log-modal-close").onclick = closeLogModal;
  document.querySelector("#log-refresh").onclick = refreshLog;
  document.querySelector("#log-copy").onclick = () => {
    const content = document.querySelector("#log-content");
    copyLogText(content.textContent);
  };
  logModal.addEventListener("click", (e) => {
    if (e.target === logModal) closeLogModal();
  });

  // 程序管理内嵌日志
  document.querySelector("#manage-log-refresh").onclick = refreshManageLog;
  // 全屏日志以弹出窗口形式打开
  document.querySelector("#manage-log-fullscreen").onclick = () => {
    if (current) openLogModal(current.id);
  };
  document.querySelector("#manage-log-copy").onclick = () => {
    const content = document.querySelector("#manage-log-content");
    copyLogText(content.textContent);
  };
  setupManageLogResize();

  // 编辑模态
  const editModal = document.querySelector("#edit-modal");
  document.querySelector("#edit-modal-close").onclick = closeEditModal;
  document.querySelector("#edit-cancel").onclick = closeEditModal;
  document.querySelector("#edit-save").onclick = (e) => {
    e.preventDefault();
    saveEdit();
  };
  document.querySelector("#edit-add-field").onclick = () => {
    document
      .querySelector("#edit-fields-body")
      .appendChild(editFieldRow({ key: "", label: "", kind: "string", default: "" }));
  };
  editModal.addEventListener("click", (e) => {
    if (e.target === editModal) closeEditModal();
  });

  // 壳操作日志弹窗
  const shellLogModal = document.querySelector("#shell-log-modal");
  const shellLogContent = document.querySelector("#shell-log-content");
  async function loadShellLog() {
    try {
      const text = await invoke("get_shell_log");
      shellLogContent.textContent = text || t("shell_log.empty");
      shellLogContent.scrollTop = shellLogContent.scrollHeight;
    } catch (e) {
      showNotice(String(e), true);
    }
  }
  function openShellLogModal() {
    shellLogModal.hidden = false;
    loadShellLog();
  }
  function closeShellLogModal() {
    shellLogModal.hidden = true;
  }
  document.querySelector("#shell-log-link").onclick = openShellLogModal;
  document.querySelector("#open-log-dir-link").onclick = async () => {
    try {
      await invoke("reveal_logs_dir");
    } catch (e) {
      showNotice(String(e), true);
    }
  };
  document.querySelector("#shell-log-modal-close").onclick = closeShellLogModal;
  document.querySelector("#shell-log-refresh").onclick = loadShellLog;
  document.querySelector("#shell-log-clear").onclick = async () => {
    try {
      await invoke("clear_shell_log");
      shellLogContent.textContent = "";
      showNotice(t("toast.op_log_cleared"));
    } catch (e) {
      showNotice(String(e), true);
    }
  };
  shellLogModal.addEventListener("click", (e) => {
    if (e.target === shellLogModal) closeShellLogModal();
  });

  registries = await invoke("get_registries");
  registryUrl = registries[0] ?? "";
  libSource = registries[0] ?? null;

  // 模板库入口按钮 + 源管理
  const libImportBtn = document.querySelector("#lib-import-local");
  if (libImportBtn) libImportBtn.onclick = importLocalFile;
  const libManageBtn = document.querySelector("#lib-manage-sources");
  if (libManageBtn) libManageBtn.onclick = openSourcesModal;
  const libCacheToggle = document.querySelector("#lib-cache-toggle");
  if (libCacheToggle) {
    libCacheToggle.onclick = () => {
      if (el.libCacheDrawer) el.libCacheDrawer.hidden = !el.libCacheDrawer.hidden;
    };
  }
  const sourcesModal = document.querySelector("#sources-modal");
  document.querySelector("#sources-modal-close").onclick = closeSourcesModal;
  document.querySelector("#sources-cancel").onclick = closeSourcesModal;
  document.querySelector("#sources-add-btn").onclick = addSourceRow;
  document.querySelector("#sources-save").onclick = saveSources;
  sourcesModal.addEventListener("click", (e) => {
    if (e.target === sourcesModal) closeSourcesModal();
  });
  await ensureLibraryFromCache();
  await refresh();
  renderLocalTemplates();

  // 搜索框只创建一次，输入时仅刷新列表（避免重建导致光标跳走）
  const libSearchInput = document.createElement("input");
  libSearchInput.placeholder = t("lib.search_ph");
  libSearchInput.value = libSearchValue;
  libSearchInput.addEventListener("input", () => {
    libSearchValue = libSearchInput.value;
    libPage = 0;
    if (manifest) renderLibrary();
  });
  el.libSearch.appendChild(libSearchInput);
  // 不自动联网检查版本：仅管理页本地状态周期性刷新(轻量、无网络)；批量页只在进入/手动刷新/操作后刷新
  setInterval(() => {
    if (view === "manage" && current) refreshStatusLocal();
  }, 15000);
  // 运行日志跟随刷新：仅程序运行期间轮询，避免无谓 IPC。
  // 同时顺带刷状态：进程一旦退出/崩溃，及时恢复「启动」按钮（原来只靠 15s 轮询，明显偏慢）。
  setInterval(() => {
    if (view !== "manage" || !current) return;
    const st = statuses.find((s) => s.id === current.id)?.status;
    refreshStatusLocal();
    if (st?.running) refreshManageLog();
  }, 3000);
});