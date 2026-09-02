const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;

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
  notice: document.querySelector("#notice"),
  manageView: document.querySelector("#manage-view"),
  batchView: document.querySelector("#batch-view"),
  libraryView: document.querySelector("#library-view"),
  batchBody: document.querySelector("#batch-body"),
  registryBar: document.querySelector("#registry-bar"),
  libStatus: document.querySelector("#lib-status"),
  libSearch: document.querySelector("#lib-search"),
  libList: document.querySelector("#lib-list"),
};

let view = "manage";
let registries = [];
let registryUrl = "";
let manifest = null;
let libSearchValue = "";

function showNotice(text, isError) {
  el.notice.textContent = text;
  el.notice.className = isError ? "error" : "ok";
}

// ---------- 程序管理 ----------

async function refresh() {
  programs = await invoke("get_programs");
  const ml = document.querySelector("#manage-log");
  if (!programs.length) {
    el.progTitle.textContent = "未导入任何程序";
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
    hint.textContent = "尚未导入任何程序\n(可在「模板库」导入)";
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
    dot.title = on ? "运行中" : "已停止";

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
    editIco.title = "编辑";
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
        ? `未安装 · ${p.repo}`
        : st.running
          ? `运行中 · v${st.local_version}`
          : `已停止 · v${st.local_version}`;
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
  // 先以本地状态即时渲染（保证启动/停止按钮正确出现），再网络补全最新版本
  await refreshStatusLocal();
  refreshStatus();
  refreshManageLog();
}

// ---------- 下载 / 更新（状态栏与批量管理共用，防重复触发）----------

const installing = new Set();

async function installProgram(id, btn) {
  if (installing.has(id)) return;
  installing.add(id);
  if (btn) {
    btn.disabled = true;
    btn.textContent = "下载中…";
  }
  syncInstallBtns(id);
  try {
    const v = await invoke("install", { programId: id });
    const p = programs.find((x) => x.id === id);
    showNotice(`已下载/更新「${p ? p.name : id}」，当前版本 ${v}`);
    if (id === current?.id) logOp(`已下载/更新，当前版本 ${v}`);
  } catch (e) {
    showNotice(String(e), true);
    if (id === current?.id) logOp(`下载/更新失败: ${e}`);
  } finally {
    installing.delete(id);
    if (btn) {
      btn.disabled = false;
      btn.textContent = btn.dataset.installed === "1" ? "更新" : "下载";
    }
    syncInstallBtns(id);
    if (view === "batch") await refreshBatch();
    else if (current?.id === id) await refreshStatus();
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
    dlBtn.textContent = busy ? "下载中…" : "下载";
  } else if (st.up_to_date || !st.latest_version) {
    // 已最新，或最新版本未知：不显示“更新”
    dlBtn.hidden = true;
  } else {
    dlBtn.hidden = false;
    dlBtn.disabled = busy;
    dlBtn.textContent = busy ? "下载中…" : "更新";
  }
}

async function renderForm() {
  el.progTitle.textContent = current.name;
  el.progSub.textContent = current.repo || "";
  el.form.innerHTML = "";
  for (const f of current.fields) {
    const row = document.createElement("div");
    row.className = "field-row";
    const label = document.createElement("label");
    label.textContent = f.label;
    label.className = "field-label";
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
            showNotice("开机启动已设置");
            logOp(check.checked ? "已开启开机启动" : "已关闭开机启动");
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
        btn.textContent = "浏览…";
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
  start.textContent = "启动";
  start.onclick = async () => {
    try {
      const st = await invoke("start_program", {
        payload: { program_id: current.id, values },
      });
      renderStatus(st);
      await refreshStatusLocal();
      logOp(`已启动`);
      refreshManageLog();
    } catch (e) {
      showNotice(String(e), true);
      logOp(`启动失败: ${e}`);
    }
  };
  el.actions.appendChild(start);

  const stop = document.createElement("button");
  stop.id = "stop-btn";
  stop.hidden = true;
  stop.textContent = "停止";
  stop.onclick = async () => {
    try {
      const st = await invoke("stop_program", { programId: current.id });
      renderStatus(st);
      await refreshStatusLocal();
      logOp("已停止");
    } catch (e) {
      showNotice(String(e), true);
      logOp(`停止失败: ${e}`);
    }
  };
  el.actions.appendChild(st);

  const restart = document.createElement("button");
  restart.id = "restart-btn";
  restart.hidden = true;
  restart.textContent = "重启";
  restart.onclick = async () => {
    try {
      const st = await invoke("restart_program", {
        payload: { program_id: current.id, values },
      });
      renderStatus(st);
      await refreshStatusLocal();
      logOp("已重启");
      refreshManageLog();
    } catch (e) {
      showNotice(String(e), true);
      logOp(`重启失败: ${e}`);
    }
  };
  el.actions.appendChild(restart);

  const logs = document.createElement("button");
  logs.textContent = "打开日志目录";
  logs.onclick = () => revealLogs();
  el.actions.appendChild(logs);

  const viewLog = document.createElement("button");
  viewLog.textContent = "查看日志";
  viewLog.onclick = () => openLogModal(current.id);
  el.actions.appendChild(viewLog);

  const edit = document.createElement("button");
  edit.textContent = "编辑";
  edit.onclick = () => openEditModal(current.id);
  el.actions.appendChild(edit);

  const hideBtn = document.createElement("button");
  hideBtn.textContent = current.hidden ? "取消隐藏" : "隐藏";
  hideBtn.onclick = async () => {
    try {
      await invoke("set_program_hidden", {
        programId: current.id,
        hidden: !current.hidden,
      });
      showNotice(current.hidden ? "已取消隐藏" : "已隐藏（在批量管理中可找回）");
      logOp(current.hidden ? "已取消隐藏" : "已隐藏");
      programs = await invoke("get_programs");
      current = programs.find((p) => p.id === current.id) || null;
      if (current?.hidden) {
        // 隐藏后跳到第一个可见程序，保持主管理列表干净
        current = programs.find((p) => !p.hidden) || null;
      }
      await refreshBatch();
      if (current) {
        await switchTo(current.id);
      } else {
        await refresh();
      }
    } catch (e) {
      showNotice(String(e), true);
    }
  };
  el.actions.appendChild(hideBtn);

  const del = document.createElement("button");
  del.textContent = "删除";
  del.className = "op-danger";
  del.onclick = () => confirmAndDelete(current.id, current.name);
  el.actions.appendChild(del);
}

async function refreshStatus() {
  if (!current) return;
  // 先用批量缓存的状态做即时渲染（若有），避免视图空白等待网络
  const cached = statuses.find((s) => s.id === current.id)?.status;
  if (cached) renderStatus(cached);
  try {
    const st = await invoke("get_status", { programId: current.id });
    renderStatus(st);
  } catch {
    // 忽略
  }
}

// 仅本地状态轮询（无网络）：程序自行退出/报错后，刷新启动按钮与运行态
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
    if (idx >= 0) statuses[idx].status = st;
  }
  const b = document.querySelector("#start-btn");
  const stop = document.querySelector("#stop-btn");
  const restart = document.querySelector("#restart-btn");
  const chips = [
    { text: `本地版本: ${st.local_version}`, cls: "" },
    { text: `最新版本: ${st.latest_version ?? "未知"}`, cls: "" },
    { text: st.installed ? "已安装" : "未安装", cls: "" },
  ].filter((c) => c.text !== "");
  if (st.autostart) {
    chips.push({ text: "开机启动", cls: "ok" });
  }
  el.chips.innerHTML = "";
  for (const c of chips) {
    const span = document.createElement("span");
    span.textContent = c.text;
    span.className = "pill" + (c.cls ? " " + c.cls : "");
    el.chips.appendChild(span);
  }
  if (b) b.hidden = st.running;
  if (stop) stop.hidden = !st.running;
  if (restart) restart.hidden = !st.running;
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
      dlBtn.textContent = busy ? "下载中…" : st.installed ? "更新" : "下载";
    }
  }
  renderSidebar();
}

async function revealLogs() {
  try {
    await invoke("reveal_logs", { programId: current.id });
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

// 本地 + 并行网络补全最新版本：用户主动操作后调用
async function refreshBatch() {
  try {
    // 第一帧：本地状态（无网络），表格立即渲染
    statuses = await invoke("batch_status_local");
    renderSidebar();
    renderBatch();
  } catch (e) {
    showNotice(String(e), true);
    return;
  }
  // 第二帧：并行补全最新版本（网络），表格静默刷新
  try {
    const full = await invoke("batch_status");
    statuses = full;
    renderSidebar();
    renderBatch();
  } catch {
    // 网络失败保留本地展示，静默忽略
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
    btn.textContent = "检查中…";
  }
  showNotice("正在检查更新…");
  try {
    const full = await invoke("batch_status");
    statuses = full;
    renderSidebar();
    renderBatch();
    showNotice("检查完成");
  } catch (e) {
    showNotice(String(e), true);
  } finally {
    if (btn) {
      btn.classList.remove("busy");
      btn.disabled = false;
      btn.textContent = "检查更新";
    }
  }
}

function renderBatch() {
  el.batchBody.innerHTML = "";
  if (!statuses.length) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 7;
    td.textContent = "尚未导入任何程序";
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
      tag.textContent = "已隐藏";
      tdName.appendChild(tag);
    }
    tr.appendChild(tdName);

    const tdLocal = document.createElement("td");
    tdLocal.textContent = s.local_version || "—";
    tr.appendChild(tdLocal);

    const tdLatest = document.createElement("td");
    tdLatest.textContent = s.latest_version ?? "未知";
    tr.appendChild(tdLatest);

    const tdState = document.createElement("td");
    tdState.textContent = !s.installed ? "未安装" : s.running ? "运行中" : "已停止";
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
        showNotice(`已更新「${item.name}」开机启动`);
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
          hide.checked ? `已隐藏「${item.name}」` : `已取消隐藏「${item.name}」`
        );
        if (item.id === current?.id && hide.checked) current = null;
        programs = await invoke("get_programs");
        if (!current) current = programs.find((p) => !p.hidden) || null;
        await refreshBatch();
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
    const dl = makeOpBtn(s.installed ? (isUpToDate ? "最新" : "更新") : "下载", () => {
      dl.dataset.installed = s.installed ? "1" : "0";
      installProgram(item.id, dl);
    });
    if (isUpToDate) dl.disabled = true;
    ops.appendChild(dl);

    const start = makeOpBtn("启动", async () => {
      try {
        await invoke("start_program", {
          payload: { program_id: item.id, values: {} },
        });
        await refreshBatch();
      } catch (e) {
        showNotice(String(e), true);
      }
    });
    ops.appendChild(start);

    const restart = makeOpBtn("重启", async () => {
      try {
        await invoke("restart_program", {
          payload: { program_id: item.id, values: {} },
        });
        await refreshBatch();
      } catch (e) {
        showNotice(String(e), true);
      }
    });
    ops.appendChild(restart);

    const stop = makeOpBtn("停止", async () => {
      try {
        await invoke("stop_program", { programId: item.id });
        await refreshBatch();
      } catch (e) {
        showNotice(String(e), true);
      }
    });
    ops.appendChild(stop);

    const logs = makeOpBtn("日志", () => openLogModal(item.id));
    ops.appendChild(logs);

    const edit = makeOpBtn("编辑", () => openEditModal(item.id));
    ops.appendChild(edit);

    const del = makeOpBtn("删除", () => confirmAndDelete(item.id, item.name));
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

function openSettings() {
  const modal = document.querySelector("#settings-modal");
  const acc = document.querySelector("#sett-accelerate");
  const hp = document.querySelector("#sett-proxy");
  acc.value = "";
  hp.value = "";
  invoke("get_proxy")
    .then((p) => {
      acc.value = p.accelerate_prefix || "";
      hp.value = p.http_proxy || "";
    })
    .catch((e) => showNotice(String(e), true));
  modal.hidden = false;
}

function closeSettings() {
  document.querySelector("#settings-modal").hidden = true;
}

async function saveSettings() {
  const acc = document.querySelector("#sett-accelerate").value;
  const hp = document.querySelector("#sett-proxy").value;
  try {
    await invoke("set_proxy", { acceleratePrefix: acc, httpProxy: hp });
    showNotice("网络设置已保存");
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
    container.textContent = "(空)";
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
  title.textContent = `日志 · ${p ? p.name : logProgramId}`;
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
    showNotice("已复制");
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
      showNotice("已复制");
    } catch {
      showNotice("复制失败", true);
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
}

function showManageLog() {
  const box = document.querySelector("#manage-log");
  if (box) box.hidden = !current || view !== "manage";
  if (!current || view !== "manage") return;
  refreshManageLog();
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
    // 操作日志 + 文件日志合并显示，操作日志置顶
    const opsText = opLogs.map((o) => `◆ ${o}`).join("\n");
    const body = opsText ? opsText + "\n\n" + logs.text : logs.text;
    renderLogBody(content, body);
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
    `确定删除程序「${name}」？\n将移除其配置、已下载二进制与日志，且不可恢复。`
  );
  if (!ok) return;
  try {
    await invoke("delete_program", { programId: id });
    showNotice(`已删除「${name}」`);
    if (current?.id === id) logOp(`已删除「${name}」`);
    programs = await invoke("get_programs");
    if (current?.id === id) {
      current = null;
      await refresh();
    } else {
      renderSidebar();
    }
    if (view === "batch") await refreshBatch();
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
  k.placeholder = "字段 key";
  k.value = f.key;
  const l = document.createElement("input");
  l.className = "l";
  l.placeholder = "标签";
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
  d.placeholder = "默认值";
  d.value = f.default ?? "";
  const del = document.createElement("button");
  del.className = "edit-field-del";
  del.type = "button";
  del.textContent = "✕";
  del.title = "删除字段";
  del.onclick = () => row.remove();
  row.append(k, l, sel, d, del);
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
      if (!k) return;
      fields.push({ key: k, kind, label: l || k, default: def });
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
    showNotice("已保存修改");
    logOp("已保存配置修改");
    const editedId = editProgramId;
    closeEditModal();
    programs = await invoke("get_programs");
    current = programs.find((p) => p.id === editedId) || current;
    if (current) {
      await switchTo(current.id);
    }
    if (view === "batch") await refreshBatch();
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
    el.progTitle.textContent = v === "batch" ? "批量管理" : "模板库";
    el.progSub.textContent =
      v === "batch" ? "所有程序统一操作" : "从远程源导入程序模板";
    el.chips.innerHTML = "";
    if (v === "batch") refreshBatch();
    else renderLibrary();
  }
}

// ---------- 模板库 ----------

function renderRegistryBar() {
  el.registryBar.innerHTML = "";
  const input = document.createElement("input");
  input.value = registryUrl;
  input.placeholder = "https://…/templates/";
  registerInput(input, (val) => (registryUrl = val));
  const sel = document.createElement("select");
  sel.style.marginLeft = "6px";
  for (const r of registries) {
    const opt = document.createElement("option");
    opt.value = r;
    opt.textContent = r;
    if (r === registryUrl) opt.selected = true;
    sel.appendChild(opt);
  }
  sel.onchange = () => {
    registryUrl = sel.value;
    input.value = sel.value;
  };
  const refresh = document.createElement("button");
  refresh.textContent = "刷新";
  refresh.onclick = async () => {
    refresh.disabled = true;
    refresh.textContent = "拉取中…";
    try {
      const m = await invoke("get_merged_manifest", { registryUrl });
      manifest = m;
      renderLibrary();
    } catch (e) {
      el.libStatus.textContent = `清单拉取失败: ${e}`;
    } finally {
      refresh.disabled = false;
      refresh.textContent = "刷新";
    }
  };
  el.registryBar.append(input, sel, refresh);
}

function registerInput(input, cb) {
  input.addEventListener("input", () => cb(input.value));
}

function renderLibrary() {
  if (!manifest) {
    el.libList.innerHTML = "尚未加载清单。点击「刷新」从远程源拉取(失败时回退本地缓存)。";
    return;
  }
  const nSources = (manifest.sources || []).length;
  const nOffline = (manifest.sources || []).filter(([, off]) => off).length;
  const nConflicts = (manifest.conflicts || []).length;
  el.libStatus.textContent =
    nOffline > 0
      ? `离线(缓存) ${nOffline}/${nSources} 源 · ${manifest.templates.length} 个模板 · 冲突 ${nConflicts}`
      : `${nSources} 个源 · ${manifest.templates.length} 个模板 · 冲突 ${nConflicts}`;

  el.libSearch.innerHTML = "";
  const s = document.createElement("input");
  s.placeholder = "搜索模板…";
  s.value = libSearchValue;
  registerInput(s, (v) => {
    libSearchValue = v;
    renderLibrary();
  });
  el.libSearch.appendChild(s);

  const kw = libSearchValue.trim().toLowerCase();
  const rows = manifest.templates.filter(([, t]) => {
    if (!kw) return true;
    const hay = t ? [t.id, t.name, t.category, t.description].join(" ").toLowerCase() : "";
    return hay.includes(kw);
  });

  el.libList.innerHTML = "";
  for (const [id, t, base] of rows) {
    const card = document.createElement("div");
    card.className = "lib-card";
    const top = document.createElement("div");
    top.className = "lib-top";
    const h = document.createElement("span");
    h.className = "lib-name";
    h.textContent = t.name;
    const cat = document.createElement("span");
    cat.className = "lib-cat";
    cat.textContent = `[${t.category}]`;
    const repo = document.createElement("span");
    repo.className = "lib-repo";
    repo.textContent = t.repo;
    const conflict = (manifest.conflicts || []).find(([cid]) => cid === id);
    if (conflict && conflict[1] > 1) {
      const mark = document.createElement("span");
      mark.className = "lib-cat lib-conflict";
      mark.textContent = `⚠ 多源×${conflict[1]}`;
      top.append(mark);
    }
    const btn = document.createElement("button");
    btn.textContent = "导入";
    btn.disabled = importing.has(id);
    btn.textContent = importing.has(id) ? "导入中…" : "导入";
    btn.onclick = async () => {
      importing.add(id);
      renderLibrary();
      try {
        await invoke("import_template", {
          registryUrl: base,
          templateId: id,
        });
        showNotice(`已导入模板「${id}」（来源: ${base}）并快照到本地配置`);
        programs = await invoke("get_programs");
        if (!current || !programs.some((p) => p.id === current.id)) {
          current = programs[0];
        }
        renderSidebar();
        if (programs.some((p) => p.id === id)) await switchTo(id);
        importing.delete(id);
        renderLibrary();
      } catch (e) {
        importing.delete(id);
        renderLibrary();
        showNotice(String(e), true);
      }
    };
    top.append(h, cat, repo, btn);
    const desc = document.createElement("div");
    desc.className = "lib-desc";
    desc.textContent = t.description;
    card.append(top, desc);
    el.libList.appendChild(card);
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
    b.title = dark ? "切换到浅色" : "切换到深色";
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
    cb.title = mode === "full" ? "收窄侧栏" : "展开侧栏";
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
        showNotice("已停止所有程序");
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
  document.querySelector("#manage-log-fullscreen").onclick = () => {
    document.querySelector("#manage-log").classList.toggle("fullscreen");
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

  registries = await invoke("get_registries");
  registryUrl = registries[0] ?? "";
  renderRegistryBar();
  await refresh();
  // 不自动联网检查版本：仅本地状态周期性刷新；版本更新走“检查更新”按钮
  setInterval(() => {
    if (view === "batch") refreshBatchLocal();
    else if (view === "manage" && current) refreshStatusLocal();
  }, 15000);
  // 运行日志跟随刷新：仅程序运行期间轮询，避免无谓 IPC
  setInterval(() => {
    if (view !== "manage" || !current) return;
    const st = statuses.find((s) => s.id === current.id)?.status;
    if (st?.running) refreshManageLog();
  }, 3000);
});