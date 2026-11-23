// 工作台：把「程序」和「手工服务」放进同一个列表，右侧详情按类型渲染。
//
// 设计约束：
// - 程序详情仍由 main.js 负责（字段表单、更新卡片、壳内日志），这里只做导航与调度页；
// - 手工服务没有 Program，详情由本文件渲染，日志走 services_log；
// - 选中服务时把 main.js 的 `current` 置空，避免它的定时器覆盖服务日志。

(function () {
  const $ = (s) => document.querySelector(s);
  const esc = (s) =>
    String(s == null ? "" : s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

  const st = {
    items: [],
    platform: "",
    filter: "all",
    query: "",
    selected: null,
    checked: new Set(),
    tab: "cfg",
    logTimer: null,
    pollTimer: null,
  };

  const stateText = (s) => t("svc.state." + s);

  function visible() {
    return st.items.filter((i) => {
      if (st.filter === "running" && i.state !== "running") return false;
      if (st.filter === "native" && i.supervisor === "managed") return false;
      if (st.filter === "update" && !i.has_update) return false;
      if (st.query) {
        const q = st.query.toLowerCase();
        return (i.name + " " + i.id + " " + (i.command || "")).toLowerCase().includes(q);
      }
      return true;
    });
  }

  function supTag(it) {
    if (it.supervisor === "managed") return `<span class="wb-tag">${esc(t("wb.sup_managed"))}</span>`;
    return `<span class="wb-tag sys">${esc(t("wb.sup_native"))}</span>`;
  }

  // ---------- 列表 ----------

  function renderList(preferId) {
    const root = $("#program-tabs");
    if (!root) return;
    const rows = visible();
    root.innerHTML = "";
    for (const it of rows) {
      const el = document.createElement("div");
      el.className = "prog-item" + (st.selected && st.selected.id === it.id ? " active" : "");

      const cb = document.createElement("input");
      cb.type = "checkbox";
      cb.className = "wb-check";
      cb.checked = st.checked.has(it.id);
      cb.onclick = (e) => {
        e.stopPropagation();
        if (cb.checked) st.checked.add(it.id);
        else st.checked.delete(it.id);
        renderBulk();
      };

      const dot = document.createElement("span");
      dot.className = "status-dot" + (it.state === "running" ? " on" : "");

      const info = document.createElement("span");
      info.className = "info";
      const name = document.createElement("span");
      name.className = "name";
      const nm = document.createElement("span");
      nm.className = "nm";
      nm.textContent = it.name;
      name.append(nm);
      if (it.has_update) {
        const tg = document.createElement("span");
        tg.className = "wb-tag upd";
        tg.textContent = "↑";
        name.appendChild(tg);
      }
      const sub = document.createElement("span");
      sub.className = "sub";
      // 调度归属放副标题：窄侧栏里名称才不会被挤掉
      sub.textContent =
        (it.state === "running" ? "● " : "○ ") +
        stateText(it.state) +
        " · " +
        (it.supervisor === "managed" ? t("wb.sup_managed") : t("wb.sup_native"));
      info.append(name, sub);

      el.append(cb, dot, info);
      el.onclick = () => select(it.id);

      if (it.kind === "program" && typeof ctxMenuAt === "function") {
        const prog = () => programs.find((p) => p.id === it.id);
        el.oncontextmenu = (e) => {
          e.preventDefault();
          e.stopPropagation();
          const p = prog();
          if (p) ctxMenuAt(e.clientX, e.clientY, progItemMenu(p));
        };
        if (typeof attachCtxLongPress === "function") {
          attachCtxLongPress(el, (x, y) => {
            const p = prog();
            if (p) ctxMenuAt(x, y, progItemMenu(p));
          });
        }
      }
      root.appendChild(el);
    }
    if (!rows.length) {
      const empty = document.createElement("div");
      empty.className = "wb-empty";
      empty.textContent = t("wb.empty");
      root.appendChild(empty);
    }

    if (preferId) {
      const target = root.querySelector('.prog-item[data-id="' + CSS.escape(preferId) + '"]');
      if (target) target.scrollIntoView({ block: "nearest" });
    }
    renderBulk();
    syncSelectAll();
  }

  function syncSelectAll() {
    const ids = visible().map((i) => i.id);
    const box = document.getElementById("wb-all");
    const label = document.getElementById("wb-all-count");
    if (box) {
      const picked = ids.filter((id) => st.checked.has(id)).length;
      box.checked = ids.length > 0 && picked === ids.length;
      box.indeterminate = picked > 0 && picked < ids.length;
      box.disabled = ids.length === 0;
    }
    if (label) label.textContent = ids.length ? st.checked.size + " / " + ids.length : "";
  }

  function toggleAll(on) {
    const ids = visible().map((i) => i.id);
    if (on) ids.forEach((id) => st.checked.add(id));
    else ids.forEach((id) => st.checked.delete(id));
    renderList();
    renderBulk();
  }

  function clearChecked() {
    st.checked.clear();
    renderList();
    renderBulk();
  }

  function renderBulk() {
    const bar = $("#wb-bulk");
    if (!bar) return;
    const n = st.checked.size;
    bar.hidden = n === 0;
    if (!n) {
      bar.innerHTML = "";
      return syncSelectAll();
    }
    bar.innerHTML =
      `<b>${esc(t("wb.bulk_selected", { n }))}</b><span class="spacer"></span>` +
      `<button data-bulk="start">${esc(t("wb.act_start"))}</button>` +
      `<button data-bulk="stop">${esc(t("wb.act_stop"))}</button>` +
      `<button data-bulk="restart">${esc(t("wb.act_restart"))}</button>` +
      `<button data-bulk="check">${esc(t("wb.act_check"))}</button>` +
      `<button data-bulk="delete" class="danger">${esc(t("wb.act_delete"))}</button>`;
    bar.querySelectorAll("[data-bulk]").forEach((b) => {
      b.onclick = () => bulk(b.dataset.bulk);
    });
    syncSelectAll();
  }

  async function bulk(act) {
    const targets = st.items.filter((i) => st.checked.has(i.id));
    if (!targets.length) return;
    if (act === "delete" && !confirm(t("wb.confirm_delete", { n: targets.length }))) return;

    // 检查更新是全局动作，一次即可
    if (act === "check") {
      try {
        await checkUpdates(null);
        await load();
      } catch (e) {
        showNotice(String(e), true);
      }
      return;
    }

    let ok = 0;
    let fail = 0;
    let firstErr = "";
    for (const it of targets) {
      try {
        if (it.kind === "program") {
          if (act === "start") await invoke("start_program", { programId: it.id, values: {} });
          else if (act === "stop") await invoke("stop_program", { programId: it.id });
          else if (act === "restart") await invoke("restart_program", { programId: it.id, values: {} });
          else if (act === "delete") await invoke("delete_program", { programId: it.id });
        } else {
          if (act === "delete") await invoke("services_remove", { id: it.id, uninstall: true });
          else await invoke("services_action", { id: it.id, action: act });
        }
        ok++;
      } catch (e) {
        fail++;
        if (!firstErr) firstErr = it.name + ": " + e;
      }
    }
    st.checked.clear();
    await load();
    if (fail) showNotice(t("wb.partial", { ok, fail }) + (firstErr ? " — " + firstErr : ""), true);
    else showNotice(t("wb.done", { n: ok }));
  }

  // ---------- 选中与详情 ----------

  async function select(id) {
    const it = st.items.find((x) => x.id === id);
    if (!it) return;
    st.selected = it;
    if (it.kind === "service" && st.tab === "upd") st.tab = "cfg";
    renderList();
    showTab(st.tab);

    if (it.kind === "program") {
      showProgramChrome();
      if (typeof switchCurrent === "function") await switchCurrent(id);
      renderSchedPane();
      renderUpdatePane();
      stopServiceLog();
      if (typeof refreshManageLog === "function") refreshManageLog();
      if (typeof isMobile === "function" && isMobile() && typeof mobGotoDetail === "function") mobGotoDetail(it);
    } else {
      showServiceChrome(it);
      renderServiceCfg(it);
      renderSchedPane();
      renderUpdatePane();
      // 让 main.js 的定时器停手：它按 current 取日志，而这里没有 Program。
      // current 是 main.js 的顶层 let，同一全局环境下可直接赋值。
      try {
        current = null;
      } catch (e) {}
      const pre = document.getElementById("manage-log-content");
      if (pre) pre.textContent = "";
      startServiceLog(it);
      if (typeof isMobile === "function" && isMobile() && typeof mobGotoDetail === "function") mobGotoDetail(it);
    }
  }

  function showProgramChrome() {
    const form = $("#field-form");
    const env = $("#env-section");
    const actions = $("#actions");
    const svc = $("#svc-form");
    if (form) form.hidden = false;
    if (env) env.hidden = true;
    if (actions) actions.hidden = false;
    if (svc) {
      svc.hidden = true;
      svc.innerHTML = "";
    }
  }

  function showServiceChrome(it) {
    const form = $("#field-form");
    const env = $("#env-section");
    const actions = $("#actions");
    const svc = $("#svc-form");
    if (form) form.hidden = true;
    if (env) env.hidden = true;
    if (actions) actions.hidden = true;
    if (svc) svc.hidden = false;

    const title = $("#prog-title");
    const sub = $("#prog-sub");
    const chips = $("#status-chips");
    if (title) title.textContent = it.name || t("wb.new_service").replace(/^＋\s*/, "");
    if (sub) sub.textContent = it.id + " · " + t("wb.manual_service");
    if (chips) {
      const cls = it.state === "running" ? "pill ok" : "pill";
      chips.innerHTML = `<span class="${cls}">${esc(stateText(it.state))}</span>`;
    }
    const dl = $("#dl-btn");
    if (dl) dl.hidden = true;
    const upd = $("#upd-toggle");
    if (upd) upd.hidden = true;
    const desc = $("#prog-desc");
    if (desc) desc.hidden = true;
  }

  // ---------- 服务：配置 ----------

  function envText(env) {
    return Object.entries(env || {})
      .map(([k, v]) => k + "=" + v)
      .join("\n");
  }

  function renderServiceCfg(it, isNew) {
    const box = $("#svc-form");
    if (!box) return;
    const head = isNew
      ? `<div class="svc-field"><label>id</label><input id="wb-f-id" placeholder="dufs"/></div>` +
        `<div class="svc-field"><label>${esc(t("wb.f_name"))}</label><input id="wb-f-name"/></div>`
      : "";
    box.innerHTML =
      head +
      `<div class="svc-field"><label>${esc(t("wb.f_cmd"))}</label><input id="wb-f-cmd" value="${esc(it.command)}"/></div>` +
      `<div class="svc-field"><label>${esc(t("wb.f_args"))}</label><textarea id="wb-f-args">${esc((it.args || []).join("\n"))}</textarea></div>` +
      `<div class="svc-field"><label>${esc(t("wb.f_cwd"))}</label><input id="wb-f-cwd" value="${esc(it.cwd || "")}"/></div>` +
      `<div class="svc-field"><label>${esc(t("wb.f_env"))}</label><textarea id="wb-f-env">${esc(envText(it.env))}</textarea></div>` +
      `<div class="svc-field"><label>${esc(t("wb.f_log"))}</label><input id="wb-f-log" value="${esc(it.log || "")}"/></div>` +
      `<div class="svc-hint">${esc(t("wb.svc_cfg_hint"))}</div>` +
      `<div class="svc-save-row">` +
      `<button class="op" id="wb-save-cfg">${esc(t("wb.save"))}</button>` +
      (isNew ? `<button class="op" id="wb-save-apply">${esc(t("wb.save_sched"))}</button>` : "") +
      (!isNew ? `<span class="spacer"></span><button class="op danger" id="wb-del-svc">${esc(t("wb.act_delete"))}</button>` : "") +
      `</div>`;
    const btn = $("#wb-save-cfg");
    if (btn) btn.onclick = () => saveService(it, { apply: false });
    const applyBtn = $("#wb-save-apply");
    if (applyBtn) applyBtn.onclick = () => saveService(it, { apply: true });
    const del = $("#wb-del-svc");
    if (del)
      del.onclick = async () => {
        if (!confirm(t("wb.confirm_delete", { n: 1 }))) return;
        try {
          const r = await invoke("services_remove", { id: it.id, uninstall: true });
          showNotice(r.message || "");
          st.selected = null;
          await load();
        } catch (e) {
          showNotice(String(e), true);
        }
      };
  }

  // ---------- 调度 ----------

  function restartOptions(cur) {
    const opts = [
      ["never", t("svc.restart.never")],
      ["on_failure", t("svc.restart.on_failure")],
      ["always", t("svc.restart.always")],
    ];
    return opts.map(([v, label]) => `<option value="${v}"${v === cur ? " selected" : ""}>${esc(label)}</option>`).join("");
  }

  function scopeOptions(cur) {
    const opts = [
      ["user", t("svc.scope.user")],
      ["system", t("svc.scope.system")],
    ];
    return opts.map(([v, label]) => `<option value="${v}"${v === cur ? " selected" : ""}>${esc(label)}</option>`).join("");
  }

  function renderSchedPane() {
    const box = $("#pane-sched");
    const it = st.selected;
    if (!box) return;
    if (!it) {
      box.innerHTML = `<div class="svc-hint">${esc(t("wb.select_hint"))}</div>`;
      return;
    }
    if (it.kind === "program") {
      const spec = it.native || {};
      const native = it.supervisor === "native";
      box.innerHTML =
        `<div class="card"><h3>${esc(t("wb.sup_native"))} / ${esc(t("wb.sup_managed"))}</h3>` +
        `<div class="seg" id="wb-seg">` +
        `<button data-sup="managed" class="${native ? "" : "on"}">${esc(t("wb.sup_managed"))}</button>` +
        `<button data-sup="native" class="${native ? "on" : ""}">${esc(t("wb.sup_native"))}</button>` +
        `</div>` +
        `<div class="note">${esc(native ? t("wb.native_hint") : t("wb.managed_hint"))}</div></div>` +
        (native ? schedForm(spec, "wb-p") : "") +
        (native
          ? `<div class="svc-save-row"><button class="op" id="wb-save-sched">${esc(t("wb.save_sched"))}</button></div>`
          : "");
      box.querySelectorAll("#wb-seg button").forEach((b) => {
        b.onclick = () => setProgramSupervisor(it, b.dataset.sup === "native");
      });
      const save = $("#wb-save-sched");
      if (save) save.onclick = () => saveProgramSched(it);
      return;
    }

    const spec = {
      delay_secs: it.delay_secs || 0,
      restart: it.restart || "never",
      scope: it.scope || "user",
      depends_on: it.depends_on || [],
      wait_timeout_secs: it.wait_timeout_secs || 60,
      ready_port: it.ready_port,
      ready_file: it.ready_file || "",
    };
    box.innerHTML =
      schedForm(spec, "wb-s") +
      `<div class="svc-save-row"><button class="op" id="wb-save-sched">${esc(t("wb.save_sched"))}</button></div>`;
    const save = $("#wb-save-sched");
    if (save) save.onclick = () => saveServiceSched(it);
  }

  function schedForm(spec, prefix) {
    return (
      `<div class="card"><h3>${esc(t("wb.tab_sched"))}</h3>` +
      `<div class="svc-field"><label>${esc(t("wb.sched_delay"))}</label><input id="${prefix}-delay" type="number" min="0" value="${spec.delay_secs || 0}"/></div>` +
      `<div class="svc-field"><label>${esc(t("wb.sched_restart"))}</label><select id="${prefix}-restart">${restartOptions(spec.restart)}</select></div>` +
      `<div class="svc-field"><label>${esc(t("wb.sched_scope"))}</label><select id="${prefix}-scope">${scopeOptions(spec.scope)}</select></div>` +
      `<div class="svc-field"><label>${esc(t("wb.sched_deps"))}</label><input id="${prefix}-deps" value="${esc((spec.depends_on || []).join(", "))}"/></div>` +
      `<div class="svc-field"><label>${esc(t("wb.sched_wait"))}</label><input id="${prefix}-wait" type="number" min="0" value="${spec.wait_timeout_secs || 60}"/></div>` +
      `<div class="svc-field"><label>${esc(t("wb.sched_rport"))}</label><input id="${prefix}-rport" value="${spec.ready_port == null ? "" : spec.ready_port}"/></div>` +
      `<div class="svc-field"><label>${esc(t("wb.sched_rfile"))}</label><input id="${prefix}-rfile" value="${esc(spec.ready_file || "")}"/></div>` +
      `</div>`
    );
  }

  function readSched(prefix) {
    const v = (id) => {
      const n = document.getElementById(prefix + "-" + id);
      return n ? n.value : "";
    };
    return {
      delaySecs: Number(v("delay")) || 0,
      restart: v("restart"),
      scope: v("scope"),
      dependsOn: v("deps")
        .split(/[,\n\s]+/)
        .map((s) => s.trim())
        .filter(Boolean),
      waitTimeoutSecs: Number(v("wait")) || 0,
      readyPort: v("rport").trim(),
      readyFile: v("rfile").trim(),
    };
  }

  async function setProgramSupervisor(it, native) {
    try {
      const r = await invoke("set_supervisor", {
        programId: it.id,
        supervisor: native ? "native" : "managed",
      });
      showNotice(r.message || "");
      // main.js 的 current/programs 是另一份缓存：不刷新的话操作区会按旧
      // supervisor 继续画启停按钮，字段自动保存也不会重新派生 native 服务。
      if (typeof reloadPrograms === "function") await reloadPrograms();
      await load();
      await select(it.id);
    } catch (e) {
      showNotice(String(e), true);
    }
  }

  async function saveProgramSched(it) {
    const s = readSched("wb-p");
    try {
      await invoke("set_supervisor", {
        programId: it.id,
        supervisor: "native",
        delaySecs: s.delaySecs,
        restart: s.restart,
        scope: s.scope,
        dependsOn: s.dependsOn,
        waitTimeoutSecs: s.waitTimeoutSecs,
        readyPort: s.readyPort,
        readyFile: s.readyFile,
      });
      showNotice(t("wb.applied"));
      // 同上：保存调度参数可能把程序切成 native，main.js 的缓存要跟上。
      if (typeof reloadPrograms === "function") await reloadPrograms();
      await load();
      await select(it.id);
    } catch (e) {
      showNotice(String(e), true);
    }
  }

  async function saveServiceSched(it) {
    const s = readSched("wb-s");
    try {
      await saveService(it, {
        apply: true,
        delay_secs: s.delaySecs,
        restart: s.restart,
        scope: s.scope,
        depends_on: s.dependsOn.join(", "),
        wait_timeout_secs: s.waitTimeoutSecs,
        ready_port: s.readyPort,
        ready_file: s.readyFile,
      });
    } catch (e) {
      showNotice(String(e), true);
    }
  }

  // ---------- 服务：保存 ----------

  async function saveService(it, over) {
    const val = (id, fb) => {
      const n = document.getElementById(id);
      return n ? n.value : fb;
    };
    const idField = document.getElementById("wb-f-id");
    const nameField = document.getElementById("wb-f-name");
    const payload = {
      id: idField ? idField.value.trim() : it.id,
      name: nameField ? nameField.value.trim() : it.name,
      description: it.description || "",
      command: over.command != null ? over.command : val("wb-f-cmd", it.command),
      args: over.args != null ? over.args : val("wb-f-args", (it.args || []).join("\n")),
      env: over.env != null ? over.env : val("wb-f-env", envText(it.env)),
      cwd: over.cwd != null ? over.cwd : val("wb-f-cwd", it.cwd || ""),
      log: over.log != null ? over.log : val("wb-f-log", it.log || ""),
      delay_secs: over.delay_secs != null ? over.delay_secs : it.delay_secs || 0,
      depends_on: over.depends_on != null ? over.depends_on : (it.depends_on || []).join(", "),
      wait_timeout_secs: over.wait_timeout_secs != null ? over.wait_timeout_secs : it.wait_timeout_secs || 60,
      ready_port: over.ready_port != null ? over.ready_port : it.ready_port == null ? "" : String(it.ready_port),
      ready_file: over.ready_file != null ? over.ready_file : it.ready_file || "",
      restart: over.restart != null ? over.restart : it.restart || "never",
      scope: over.scope != null ? over.scope : it.scope || "user",
      enabled: over.enabled != null ? over.enabled : it.enabled !== false,
      apply: !!over.apply,
    };
    if (!payload.id) {
      showNotice("id 不能为空", true);
      return;
    }
    if (!payload.command) {
      showNotice(t("wb.f_cmd") + " 不能为空", true);
      return;
    }
    const r = await invoke("services_save", { payload });
    showNotice(r.message || t("wb.saved"));
    await load();
    await select(payload.id);
  }

  async function newService() {
    // 表单在 #manage-view 内：视图停在模板库 / 日志中心时父容器是隐藏的，
    // 不先切回来，点「新建服务」就像什么都没发生。
    const mv = document.getElementById("manage-view");
    if (mv && mv.hidden && typeof switchView === "function") switchView("manage");
    const it = {
      kind: "service",
      id: "",
      name: "",
      description: "",
      command: "",
      args: [],
      env: {},
      cwd: "",
      log: "",
      delay_secs: 0,
      restart: "never",
      scope: "user",
      depends_on: [],
      wait_timeout_secs: 60,
      ready_port: null,
      ready_file: "",
      enabled: true,
      state: "not_installed",
    };
    st.selected = it;
    renderList();
    showServiceChrome(it);
    st.tab = "cfg";
    showTab("cfg");
    renderServiceCfg(it, true);
    renderSchedPane();
    const n = document.getElementById("wb-f-id");
    if (n) n.focus();
  }

  // ---------- 更新页 ----------

  function renderUpdatePane() {
    const box = $("#pane-upd");
    const prog = $("#upd-program");
    const empty = $("#upd-empty");
    const it = st.selected;
    if (!box || !prog || !empty) return;

    // 只切显隐，不用 innerHTML 覆盖：`#upd-settings` 被删掉一次，
    // main.js 缓存的引用就永久失效，更新页再也画不出来。
    if (!it || it.kind === "service") {
      prog.hidden = true;
      empty.hidden = false;
      empty.textContent = it ? t("wb.upd_na") : t("wb.select_hint");
      return;
    }
    prog.hidden = false;
    empty.hidden = true;
    if (typeof renderUpdateSettings === "function") renderUpdateSettings();
    const strip = document.getElementById("upd-settings");
    // 进入「更新」页就是来看它的，折叠状态不该把它藏起来
    if (strip) strip.hidden = false;
  }

  // ---------- tab ----------

  function showTab(name) {
    st.tab = name;
    document.querySelectorAll("#wb-tabs .wb-tab").forEach((b) => {
      b.classList.toggle("on", b.dataset.wbtab === name);
    });
    const panes = { cfg: $("#pane-cfg"), sched: $("#pane-sched"), upd: $("#pane-upd") };
    for (const [k, node] of Object.entries(panes)) {
      if (node) node.hidden = k !== name;
    }
    // 更新卡片在切换时可能已被 main.js 按 program 重新渲染
    if (name === "upd") renderUpdatePane();
  }

  // ---------- 日志 ----------

  function startServiceLog(it) {
    stopServiceLog();
    const tick = async () => {
      if (!st.selected || st.selected.id !== it.id) return;
      try {
        const r = await invoke("services_log", { id: it.id, tail: 400 });
        const pre = document.getElementById("manage-log-content");
        if (pre) {
          pre.textContent = (r.lines || []).join("\n");
          pre.scrollTop = pre.scrollHeight;
        }
        const hint = document.getElementById("manage-log-hint");
        if (hint) hint.textContent = (r.size / 1024).toFixed(1) + " KB";
      } catch (e) {
        /* 日志读不到不该打断界面 */
      }
    };
    tick();
    st.logTimer = setInterval(tick, 2000);
  }

  function stopServiceLog() {
    if (st.logTimer) clearInterval(st.logTimer);
    st.logTimer = null;
  }

  // ---------- 数据 ----------

  async function load() {
    try {
      const r = await invoke("workspace_list");
      st.items = r.items || [];
      st.platform = r.platform || "";
      const keep = st.selected ? st.selected.id : null;
      if (keep) st.selected = st.items.find((i) => i.id === keep) || null;
      renderList();
      return st.items;
    } catch (e) {
      showNotice(String(e), true);
      return [];
    }
  }

  function onShow() {
    load();
    if (!st.pollTimer) {
      st.pollTimer = setInterval(() => {
        if (!document.hidden) load();
      }, 5000);
    }
  }

  function onHide() {
    stopServiceLog();
    if (st.pollTimer) clearInterval(st.pollTimer);
    st.pollTimer = null;
  }

  // ---------- 移动端 ----------

  function renderMobCards() {
    const root = document.getElementById("mob-cards");
    if (!root) return;
    root.innerHTML = "";
    const rows = visible();
    if (!rows.length) {
      const empty = document.createElement("div");
      empty.className = "mob-card mob-empty";
      empty.textContent = t("ui.empty");
      root.appendChild(empty);
      return;
    }
    for (const it of rows) {
      const card = document.createElement("div");
      card.className = "mob-card";
      const ico = document.createElement("span");
      ico.className = "mob-card-ico";
      ico.textContent = (it.name || "?").slice(0, 1).toUpperCase();
      const body = document.createElement("span");
      body.className = "mob-card-body";
      const nm = document.createElement("span");
      nm.className = "mob-card-name";
      nm.textContent = it.name;
      const sub = document.createElement("span");
      sub.className = "mob-card-sub";
      sub.textContent =
        (it.state === "running" ? "● " : "○ ") + stateText(it.state) + (it.has_update ? " · ↑ " + it.latest_version : "");
      body.append(nm, sub);
      const dot = document.createElement("span");
      dot.className = "mob-card-dot" + (it.state === "running" ? " on" : "");
      card.append(dot, ico, body);
      card.onclick = () => select(it.id);
      root.appendChild(card);
    }
  }

  // ---------- 绑定 ----------

  // 自己把列表拉起来：main.js 的 boot() 在脚本解析顺序上可能早于本文件，
  // 它那一刻拿不到 window.usWorkbench，不能指望它触发首次加载。
  function autostart() {
    load();
  }
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", autostart);
  } else {
    autostart();
  }

  document.addEventListener("DOMContentLoaded", () => {
    const q = document.getElementById("wb-q");
    if (q) {
      q.oninput = () => {
        st.query = q.value.trim();
        renderList();
      };
    }
    const chips = document.getElementById("wb-chips");
    if (chips) {
      chips.onclick = (e) => {
        const b = e.target.closest(".wb-chip");
        if (!b) return;
        st.filter = b.dataset.wbf;
        chips.querySelectorAll(".wb-chip").forEach((c) => c.classList.toggle("on", c === b));
        renderList();
      };
    }
    const all = document.getElementById("wb-all");
    if (all) all.onchange = () => toggleAll(all.checked);
    // 「＋ 新建」一个入口、两种对象：程序手动填参数，服务写一条命令。
    // 模板库是侧栏的独立入口，不要塞进这里。
    const add = document.getElementById("wb-new");
    const menu = document.getElementById("wb-newmenu");
    if (add && menu) {
      add.onclick = (e) => {
        e.stopPropagation();
        menu.hidden = !menu.hidden;
      };
      menu.onclick = (e) => {
        const b = e.target.closest("[data-new]");
        if (!b) return;
        menu.hidden = true;
        if (b.dataset.new === "program") openEditModal(null);
        else newService();
      };
      document.addEventListener("click", (e) => {
        if (!menu.hidden && !e.target.closest(".wb-new-wrap")) menu.hidden = true;
      });
    }
    const tabs = document.getElementById("wb-tabs");
    if (tabs) {
      tabs.onclick = (e) => {
        const b = e.target.closest(".wb-tab");
        if (b) showTab(b.dataset.wbtab);
      };
    }
  });

  window.usWorkbench = {
    load,
    renderList,
    renderMobCards,
    select,
    newService,
    showTab,
    onShow,
    onHide,
    isServiceSelected: () => !!(st.selected && st.selected.kind === "service"),
    selectedId: () => (st.selected ? st.selected.id : null),
    items: () => st.items,
  };
})();
