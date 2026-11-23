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
    sortKey: "",
    sortDir: 1,
    dense: false,
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

  // ---------- 列表（表格） ----------

  // 列定义：k 同时是排序键。名称列给百分比、其余给像素，行尾再加一个空的
  // 「填充列」吃掉剩余宽度——否则 table-layout:fixed 会把多出来的空间全塞给名称列，
  // 窄屏（检查器收成抽屉）时名称列会撑到 600px 以上，右边空一大块。
  const COLS = [
    { k: "state", label: "wb.col_state", w: "88px" },
    { k: "name", label: "wb.col_name", w: "280px" },
    { k: "category", label: "wb.col_cat", w: "92px" },
    { k: "version", label: "wb.col_ver", w: "108px" },
    { k: "port", label: "wb.col_port", w: "64px" },
    { k: "supervisor", label: "wb.col_sup", w: "78px" },
    { k: "autostart", label: "wb.col_auto", w: "46px" },
    { k: "description", label: "wb.col_desc", w: "180px" },
  ];

  // 按 id 派生稳定色：刷新后颜色不跳，和图标首字母一起构成记忆点。
  const HUES = ["#4f8cff", "#8b5cf6", "#0ea5e9", "#f59e0b", "#10b981", "#ec4899", "#ef4444", "#6366f1", "#14b8a6", "#a855f7"];
  function hueOf(id) {
    let h = 0;
    for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
    return HUES[h % HUES.length];
  }

  function sortRows(rows) {
    if (!st.sortKey) return rows;
    const k = st.sortKey;
    const rank = { running: 0, error: 1, stopped: 2, not_installed: 3, unknown: 4 };
    return rows.slice().sort((a, b) => {
      let x = a[k];
      let y = b[k];
      if (k === "state") {
        x = rank[x] ?? 9;
        y = rank[y] ?? 9;
      }
      if (typeof x === "number" && typeof y === "number") return (x - y) * st.sortDir;
      return String(x == null ? "" : x).localeCompare(String(y == null ? "" : y), "zh") * st.sortDir;
    });
  }

  function statePill(it) {
    const cls =
      it.state === "running" ? "running" : it.state === "error" ? "error" : it.state === "not_installed" ? "missing" : "stopped";
    return `<span class="wb-pill ${cls}"><i></i>${esc(stateText(it.state))}</span>`;
  }

  function rowHTML(it) {
    const sel = st.selected && st.selected.id === it.id;
    const picked = st.checked.has(it.id);
    const upd = it.has_update && it.latest_version;
    const sup =
      it.kind === "service" ? t("wb.manual_service") : it.supervisor === "native" ? t("wb.sup_native") : t("wb.sup_managed");
    const cat = it.category || (it.kind === "service" ? t("wb.manual_service") : "—");
    const ver = it.version && it.version !== "-" ? "v" + it.version : "—";
    return (
      `<tr class="wb-row${sel ? " on" : ""}${picked ? " picked" : ""}" data-id="${esc(it.id)}">` +
      `<td class="wb-cb"><input type="checkbox" data-check="${esc(it.id)}"${picked ? " checked" : ""}></td>` +
      `<td>${statePill(it)}</td>` +
      `<td><div class="wb-name"><span class="wb-ava" style="background:${hueOf(it.id)}">${esc((it.name || "?").slice(0, 1).toUpperCase())}</span>` +
      `<span class="wb-nmwrap"><b>${esc(it.name)}</b><i>${esc(it.id)}</i></span></div></td>` +
      `<td><span class="wb-cat">${esc(cat)}</span></td>` +
      `<td><span class="wb-ver">${esc(ver)}</span>${upd ? `<span class="wb-upd">↑ ${esc(it.latest_version)}</span>` : ""}</td>` +
      `<td><span class="wb-port">${it.port ? ":" + it.port : "—"}</span></td>` +
      `<td><span class="wb-sup${it.supervisor === "native" ? " sys" : ""}">${esc(sup)}</span></td>` +
      `<td>${
        it.kind === "service"
          ? `<span class="wb-port" title="${esc(t("wb.manual_service"))}">—</span>`
          : `<span class="wb-sw${it.autostart ? " on" : ""}" data-auto="${esc(it.id)}" role="switch" aria-checked="${it.autostart ? "true" : "false"}"></span>`
      }</td>` +
      `<td><span class="wb-desc" title="${esc(it.description || "")}">${esc(it.description || "—")}</span></td>` +
      `<td class="wb-fill"></td>` +
      `</tr>`
    );
  }

  function renderHead() {
    const head = document.getElementById("wb-head");
    if (!head) return;
    head.innerHTML =
      `<th class="wb-cb"></th>` +
      COLS.map((c) => {
        const sortable = c.k !== "autostart" && c.k !== "description";
        const sorted = st.sortKey === c.k;
        return (
          `<th${c.w ? ` style="width:${c.w}"` : ""} class="${sortable ? "sortable" : ""}${sorted ? " sorted" : ""}"` +
          `${sortable ? ` data-sort="${c.k}"` : ""}>${esc(t(c.label))}` +
          `${sortable ? `<span class="wb-arrow">${sorted ? (st.sortDir > 0 ? "▲" : "▼") : "↕"}</span>` : ""}</th>`
        );
      }).join("") +
      `<th class="wb-fill"></th>`;
  }

  function applyDensity() {
    const tbl = document.getElementById("wb-table");
    if (tbl) tbl.classList.toggle("dense", !!st.dense);
    const den = document.getElementById("wb-density");
    if (den) den.textContent = st.dense ? t("wb.density_comfy") : t("wb.density_dense");
    try {
      localStorage.setItem("us.wb.dense", st.dense ? "1" : "0");
    } catch (e) {}
  }

  function renderList(preferId) {
    const body = document.getElementById("wb-body");
    if (!body) return;
    renderHead();
    // 语言包是异步加载的：bindTable 首次跑时 dict 还是空的，
    // 所以把密度按钮的文案刷新挂在每次 renderList 上（切语言也会走到这里）。
    applyDensity();
    const rows = sortRows(visible());
    body.innerHTML = rows.map(rowHTML).join("");

    const empty = document.getElementById("wb-empty");
    if (empty) {
      empty.hidden = rows.length > 0;
      empty.textContent = t("wb.empty");
    }
    const cnt = document.getElementById("wb-count");
    if (cnt) {
      const running = st.items.filter((i) => i.state === "running").length;
      // 过滤时先给「命中数 / 总数」，否则筛选后数字不变，看不出筛掉了多少
      const head = rows.length === st.items.length ? String(st.items.length) : rows.length + " / " + st.items.length;
      cnt.textContent = head + " · " + t("wb.running") + " " + running;
    }
    if (preferId) {
      const target = body.querySelector(`tr[data-id="${CSS.escape(preferId)}"]`);
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
    // 批量删除会改列表成员：走 reloadPrograms 一并刷新 main.js 的 programs/statuses，
    // 否则详情区还拿着已经删掉的程序。
    if (typeof reloadPrograms === "function") await reloadPrograms();
    else await load();
    if (fail) showNotice(t("wb.partial", { ok, fail }) + (firstErr ? " — " + firstErr : ""), true);
    else showNotice(t("wb.done", { n: ok }));
  }

  // ---------- 选中与详情 ----------

  // openDrawer 只有「用户主动点行」才传 true：首屏自动选中第一条时不该把抽屉弹出来。
  async function select(id, openDrawer) {
    const it = st.items.find((x) => x.id === id);
    if (!it) return;
    st.selected = it;
    if (it.kind === "service" && st.tab === "upd") st.tab = "cfg";
    renderList();
    showTab(st.tab);
    // 窄屏（<1100）检查器变成抽屉：点行才滑出，桌面端常驻不需要这个 class。
    if (openDrawer && window.matchMedia("(max-width: 1100px)").matches) document.body.classList.add("wb-insp-open");

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
      // reloadPrograms 同时刷侧栏（st.items），不必再 load() 一次。
      if (typeof reloadPrograms === "function") await reloadPrograms();
      else await load();
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
      else await load();
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

  // 两行列表 + 分类小标题：和桌面表格同源（同一份 visible()/sortRows()），
  // 只是把列换成「名称 / 端口·版本」两行，状态收成一个圆点。
  function renderMobCards() {
    const root = document.getElementById("mob-cards");
    if (!root) return;
    const rows = sortRows(visible());
    if (!rows.length) {
      root.innerHTML = `<div class="mob-empty">${esc(t("ui.empty"))}</div>`;
      return;
    }
    const groups = new Map();
    for (const it of rows) {
      const key = it.category || (it.kind === "service" ? t("wb.manual_service") : t("ui.program"));
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(it);
    }
    let html = "";
    for (const [cat, list] of groups) {
      html += `<div class="mob-group">${esc(cat)}<span>${list.length}</span></div>`;
      html += list
        .map((it) => {
          const sub = [
            it.port ? ":" + it.port : "",
            it.version && it.version !== "-" ? "v" + it.version : "",
            it.has_update && it.latest_version ? "↑ " + it.latest_version : "",
          ]
            .filter(Boolean)
            .join(" · ");
          return (
            `<div class="mob-row" data-id="${esc(it.id)}">` +
            `<span class="mob-row-ava" style="background:${hueOf(it.id)}">${esc((it.name || "?").slice(0, 1).toUpperCase())}</span>` +
            `<span class="mob-row-body"><b>${esc(it.name)}</b><i>${esc(sub || it.id)}</i></span>` +
            `<span class="mob-row-dot ${it.state}"></span>` +
            `<span class="mob-row-chev">›</span>` +
            `</div>`
          );
        })
        .join("");
    }
    root.innerHTML = html;
  }

  // ---------- 绑定 ----------

  // 表格交互只绑一次，行内容重绘不影响事件。
  function bindTable() {
    const table = document.getElementById("wb-table");
    if (!table) return;
    table.addEventListener("click", (e) => {
      const th = e.target.closest("th[data-sort]");
      if (th) {
        const k = th.dataset.sort;
        st.sortDir = st.sortKey === k ? -st.sortDir : 1;
        st.sortKey = k;
        renderList();
        return;
      }
      const sw = e.target.closest("[data-auto]");
      if (sw) {
        e.stopPropagation();
        const it = st.items.find((x) => x.id === sw.dataset.auto);
        if (!it) return;
        const next = !it.autostart;
        it.autostart = next; // 乐观更新，失败再拉回真实值
        renderList();
        invoke("set_autostart", { programId: it.id, enabled: next })
          .then(() => load())
          .catch((err) => {
            showNotice(String(err), true);
            load();
          });
        return;
      }
      if (e.target.closest("input[data-check]")) return;
      const tr = e.target.closest("tr[data-id]");
      if (tr) select(tr.dataset.id, true);
    });
    table.addEventListener("change", (e) => {
      const cb = e.target.closest("input[data-check]");
      if (!cb) return;
      const id = cb.dataset.check;
      if (cb.checked) st.checked.add(id);
      else st.checked.delete(id);
      const tr = cb.closest("tr");
      if (tr) tr.classList.toggle("picked", cb.checked);
      renderBulk();
      syncSelectAll();
    });
    // 行右键 = 程序菜单（编辑/复制/删除/打开目录…），服务没有这套动作。
    table.addEventListener("contextmenu", (e) => {
      const tr = e.target.closest("tr[data-id]");
      if (!tr) return;
      const it = st.items.find((x) => x.id === tr.dataset.id);
      if (!it || it.kind !== "program") return;
      const p = typeof programs !== "undefined" ? programs.find((x) => x.id === it.id) : null;
      if (p && typeof ctxMenuAt === "function") {
        e.preventDefault();
        ctxMenuAt(e.clientX, e.clientY, progItemMenu(p));
      }
    });

    const den = document.getElementById("wb-density");
    try {
      st.dense = localStorage.getItem("us.wb.dense") === "1";
    } catch (e) {}
    applyDensity();
    if (den) {
      den.onclick = () => {
        st.dense = !st.dense;
        applyDensity();
      };
    }
    const close = document.getElementById("wb-insp-close");
    if (close) close.onclick = () => document.body.classList.remove("wb-insp-open");
  }

  function bindMob() {
    const list = document.getElementById("mob-cards");
    if (list) {
      list.addEventListener("click", (e) => {
        const row = e.target.closest(".mob-row");
        if (row) select(row.dataset.id, true);
      });
    }
    const q = document.getElementById("mob-q");
    if (q) {
      q.oninput = () => {
        st.query = q.value.trim();
        renderMobCards();
      };
    }
    const chips = document.getElementById("mob-chips");
    if (chips) {
      chips.onclick = (e) => {
        const b = e.target.closest(".mob-chip");
        if (!b) return;
        st.filter = b.dataset.mf;
        chips.querySelectorAll(".mob-chip").forEach((c) => c.classList.toggle("on", c === b));
        renderMobCards();
      };
    }
  }

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
    bindTable();
    bindMob();
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
