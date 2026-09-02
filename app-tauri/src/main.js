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

function setRunDot(running) {
  const dot = document.querySelector("#run-dot");
  if (!dot) return;
  dot.className = "status-dot" + (running ? " on" : "");
  dot.title = running ? "运行中" : "已停止";
}

// ---------- 程序管理 ----------

async function refresh() {
  programs = await invoke("get_programs");
  if (!programs.length) {
    el.progTitle.textContent = "未导入任何程序";
    el.progSub.textContent = "";
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
    name.textContent = p.name;
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
  renderForm();
  await refreshStatus();
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
  const dl = document.createElement("button");
  dl.textContent = "下载 / 更新";
  dl.onclick = async () => {
    dl.disabled = true;
    dl.textContent = "下载中…";
    try {
      const v = await invoke("install", { programId: current.id });
      showNotice(`下载/更新完成，当前版本 ${v}`);
    } catch (e) {
      showNotice(String(e), true);
    } finally {
      dl.disabled = false;
      dl.textContent = "下载 / 更新";
      await refreshStatus();
    }
  };
  el.actions.appendChild(dl);

  const start = document.createElement("button");
  start.id = "start-btn";
  start.textContent = "启动";
  start.onclick = async () => {
    try {
      const st = await invoke("start_program", {
        payload: { program_id: current.id, values },
      });
      renderStatus(st);
      showNotice("已启动");
    } catch (e) {
      showNotice(String(e), true);
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
      showNotice("已停止");
    } catch (e) {
      showNotice(String(e), true);
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
      showNotice("已重启");
    } catch (e) {
      showNotice(String(e), true);
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
      programs = await invoke("get_programs");
      const wasHidden = current.hidden;
      current = programs.find((p) => p.id === current.id) || null;
      if (current && !wasHidden) {
        // 隐藏后跳到侧栏第一个可见程序，保持主管理列表干净
        const firstVisible = programs.find((p) => !p.hidden);
        current = firstVisible || current;
      }
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
  try {
    const st = await invoke("get_status", { programId: current.id });
    renderStatus(st);
  } catch (e) {
    // 忽略
  }
}

function renderStatus(st) {
  const b = document.querySelector("#start-btn");
  const stop = document.querySelector("#stop-btn");
  const restart = document.querySelector("#restart-btn");
  setRunDot(st.running);
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

async function refreshBatch() {
  try {
    statuses = await invoke("batch_status");
    renderSidebar();
    renderBatch();
  } catch (e) {
    showNotice(String(e), true);
  }
}

function renderBatch() {
  el.batchBody.innerHTML = "";
  if (!statuses.length) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 6;
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

    const tdOps = document.createElement("td");
    const ops = document.createElement("span");
    ops.className = "batch-ops";

    // 下载 / 启动 / 重启 / 停止
    const dl = makeOpBtn("下载", async () => {
      try {
        await invoke("install", { programId: item.id });
        showNotice(`已下载「${item.name}」`);
        await refreshBatch();
      } catch (e) {
        showNotice(String(e), true);
      }
    });
    ops.appendChild(dl);

    const start = makeOpBtn("启动", async () => {
      try {
        await invoke("start_program", {
          payload: { program_id: item.id, values: {} },
        });
        showNotice(`已启动「${item.name}」`);
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
        showNotice(`已重启「${item.name}」`);
        await refreshBatch();
      } catch (e) {
        showNotice(String(e), true);
      }
    });
    ops.appendChild(restart);

    const stop = makeOpBtn("停止", async () => {
      try {
        await invoke("stop_program", { programId: item.id });
        showNotice(`已停止「${item.name}」`);
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

    const hide = makeOpBtn(item.hidden ? "取消隐藏" : "隐藏", async () => {
      try {
        await invoke("set_program_hidden", {
          programId: item.id,
          hidden: !item.hidden,
        });
        showNotice(item.hidden ? `已取消隐藏「${item.name}」` : `已隐藏「${item.name}」`);
        if (item.id === current?.id) await switchTo(item.id);
        await refreshBatch();
        renderSidebar();
      } catch (e) {
        showNotice(String(e), true);
      }
    });
    ops.appendChild(hide);

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

// ---------- 日志查看 ----------

let logProgramId = null;
let logTab = "out";

function openLogModal(id) {
  logProgramId = id;
  logTab = "out";
  document.querySelector("#log-modal").hidden = false;
  setLogTab();
  refreshLog();
}

function setLogTab() {
  const out = document.querySelector("#log-tab-out");
  const err = document.querySelector("#log-tab-err");
  out.classList.toggle("active", logTab === "out");
  err.classList.toggle("active", logTab === "err");
}

async function refreshLog() {
  const content = document.querySelector("#log-content");
  const title = document.querySelector("#log-modal-title");
  const p = programs.find((x) => x.id === logProgramId);
  title.textContent = `日志 · ${p ? p.name : logProgramId}`;
  try {
    const logs = await invoke("get_logs", { programId: logProgramId });
    content.textContent = (logTab === "out" ? logs.out : logs.err) || "(空)";
  } catch (e) {
    content.textContent = String(e);
  }
}

function closeLogModal() {
  document.querySelector("#log-modal").hidden = true;
  logProgramId = null;
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
  const dot = document.querySelector("#run-dot");
  if (dot) dot.style.display = v === "manage" ? "" : "none";
  if (v === "manage") {
    if (current) switchTo(current.id);
    if (dot && statuses.length) {
      const st = statuses.find((s) => s.id === current.id)?.status;
      setRunDot(st?.running ?? false);
    }
  } else {
    el.progTitle.textContent = v === "batch" ? "批量管理" : "模板库";
    el.progSub.textContent =
      v === "batch" ? "所有程序统一操作" : "从远程源导入程序模板";
    el.chips.innerHTML = "";
    if (dot) dot.className = "status-dot";
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
  if (refreshBtn) refreshBtn.onclick = () => refreshBatch();
  const sa = document.querySelector("#batch-stop-all");
  if (sa)
    sa.onclick = async () => {
      try {
        await invoke("stop_all");
        showNotice("已停止所有程序");
        await refreshBatch();
      } catch (e) {
        showNotice(String(e), true);
      }
    };

  // 日志模态
  const logModal = document.querySelector("#log-modal");
  document.querySelector("#log-modal-close").onclick = closeLogModal;
  document.querySelector("#log-tab-out").onclick = () => {
    logTab = "out";
    setLogTab();
    refreshLog();
  };
  document.querySelector("#log-tab-err").onclick = () => {
    logTab = "err";
    setLogTab();
    refreshLog();
  };
  document.querySelector("#log-refresh").onclick = refreshLog;
  logModal.addEventListener("click", (e) => {
    if (e.target === logModal) closeLogModal();
  });

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
  setInterval(() => {
    if (view === "manage" && current) refreshStatus();
    if (view === "batch") refreshBatch();
  }, 15000);
});