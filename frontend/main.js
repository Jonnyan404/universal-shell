// Universal Shell 浏览器端 SPA（F-1 骨架版）
// 与桌面窗口共用同一份代码；一切能力经 api.js 走 /api/rpc。

let programs = [];
let current = null;
let values = {};
let statuses = [];
let logTimer = null;

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
async function switchCurrent(id) {
  current = programs.find((p) => p.id === id) || null;
  if (!current) return;
  document.querySelector("#manage-view").hidden = false;
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

  setInterval(refreshAllStatuses, 3000);
  setInterval(() => {
    if (current) refreshManageLog();
  }, 3000);
}

boot().catch((e) => {
  showNotice(String(e), true);
});