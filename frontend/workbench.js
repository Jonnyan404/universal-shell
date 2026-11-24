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
    tab: "ov",
    logTimer: null,
    pollTimer: null,
    sortKey: "",
    sortDir: 1,
    dense: false,
    // 详情页概览要的内存曲线与事件时间线。跟列表分开取：这两样比列表项重得多，
    // 40 个程序全带上会把 workspace_list 撑到几百 KB。
    insight: null,
    insightFor: null,
  };

  // main.js 的 Program 没有 state 字段，拼出来会是 svc.state.undefined，
  // 直接兜到「未知」而不是把 key 露在界面上。
  const stateText = (s) => t("svc.state." + (s || "unknown"));

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
  // 列集合对齐 06 原型：状态 / 名称 / 分类 / 版本 / 端口 / 调度 / 自启 / 运行时长 / 操作。
  // 描述不进表格——它在检查器概览里，表格里放它会把「运行时长 / 操作」挤掉。
  const COLS = [
    { k: "state", label: "wb.col_state", w: "88px" },
    { k: "name", label: "wb.col_name", w: "260px" },
    { k: "category", label: "wb.col_cat", w: "92px" },
    { k: "version", label: "wb.col_ver", w: "108px" },
    { k: "port", label: "wb.col_port", w: "64px" },
    { k: "supervisor", label: "wb.col_sup", w: "78px" },
    { k: "autostart", label: "wb.col_auto", w: "46px" },
    { k: "uptime", label: "wb.col_uptime", w: "86px" },
    // 118px：4 个 22px 按钮 + 间距 + 单元格左右内边距。原来是 92px（只够 3 个），
    // 加了「打开网站」↗ 之后最右的 ⋯ 会溢出到填充列下面，被那一格吃掉点击。
    { k: "actions", label: "wb.col_act", w: "118px" },
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
    // 外部接管：进程是上次会话残留的，壳只有租约、没有句柄。
    // 必须显示成独立的第三态，否则用户看到「运行中」会以为一切正常，
    // 实际上停止/重启/退出码/内存曲线全都不在壳手里。
    if (it.external_pid) {
      return (
        `<span class="wb-pill external" title="${esc(t("wb.ext_tip", { pid: it.external_pid }))}">` +
        `<i></i>${esc(t("wb.state_external"))}</span>`
      );
    }
    const cls =
      it.state === "running" ? "running" : it.state === "error" ? "error" : it.state === "not_installed" ? "missing" : "stopped";
    return `<span class="wb-pill ${cls}"><i></i>${esc(stateText(it.state))}</span>`;
  }

  // 运行时长：秒 → 「3天4小时」这种口语写法。拿不到（系统调度 / 未运行）给「—」。
  function fmtUptime(s) {
    if (s == null || s === "" || Number.isNaN(Number(s))) return "—";
    const n = Number(s);
    if (n < 60) return t("wb.up_just");
    const d = Math.floor(n / 86400);
    const h = Math.floor((n % 86400) / 3600);
    const m = Math.floor((n % 3600) / 60);
    if (d) return d + t("wb.up_d") + (h ? h + t("wb.up_h") : "");
    if (h) return h + t("wb.up_h") + (m ? m + t("wb.up_m") : "");
    return m + t("wb.up_m");
  }

  // 行内快捷操作：打开网站 / 启停 / 重启 / 更多（复用右键菜单）
  function actHTML(it) {
    const running = it.state === "running";
    const ready = it.state !== "not_installed";
    const id = esc(it.id);
    const label = running ? t("act.stop") : t("act.start");
    // 有站点才多一个 ↗：大部分程序不是网页服务，无条件加会把操作列撑爆。
    // 用 main.js 的 webUrl 判断，两处口径一致。
    const p = typeof programs !== "undefined" && programs ? programs.find((x) => x.id === it.id) : null;
    const site = p && typeof webUrl === "function" && webUrl(p) ? "1" : "";
    return (
      (site
        ? `<button class="wb-act" data-act="site" data-id="${id}" title="${esc(t("act.open_site"))}"${running ? "" : " disabled"}>↗</button>`
        : "") +
      `<button class="wb-act${running ? " stop" : " play"}" data-act="${running ? "stop" : "start"}" data-id="${id}"` +
      ` title="${esc(label)}"${ready ? "" : " disabled"}>${running ? "■" : "▶"}</button>` +
      `<button class="wb-act" data-act="restart" data-id="${id}" title="${esc(t("act.restart"))}"${ready ? "" : " disabled"}>↻</button>` +
      `<button class="wb-act" data-act="more" data-id="${id}" title="${esc(t("wb.more"))}">⋯</button>`
    );
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
      `<td><span class="wb-uptime">${esc(fmtUptime(it.uptime))}</span></td>` +
      `<td class="wb-actcell">${actHTML(it)}</td>` +
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
        const sortable = c.k !== "autostart" && c.k !== "actions";
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
    renderInspHead(it);
    // 内存曲线 / 事件时间线单独取：列表项只带一个 mem_kb，曲线和事件重得多
    st.insight = null;
    st.insightFor = null;
    loadInsight(it.id);
    showTab(st.tab);
    // 窄屏（<1100）检查器变成抽屉：点行才滑出，桌面端常驻不需要这个 class。
    if (openDrawer && window.matchMedia("(max-width: 1100px)").matches) document.body.classList.add("wb-insp-open");
    // 点行永远重新打开详情：否则宽屏关掉之后没有恢复入口，会卡死在「只有列表」。
    document.body.classList.remove("wb-insp-closed");

    if (it.kind === "program") {
      showProgramChrome();
      // 手机端先跳二级页再拉数据：switchCurrent 里串着 get_values / 状态 / 版本几个
      // RPC，慢网下要好几秒，用户点了行会以为没反应。
      if (typeof isMobile === "function" && isMobile() && typeof mobGotoDetail === "function") mobGotoDetail(it);
      if (typeof switchCurrent === "function") await switchCurrent(id);
      renderSchedPane();
      renderUpdatePane();
      stopServiceLog();
      if (typeof refreshManageLog === "function") refreshManageLog();
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
    // 环境变量块常显：没有 env 时显示空态说明，而不是整块消失
    // （隐藏会让人以为程序没有 env，实际可能是字段没填全、派生结果为空）
    if (env) env.hidden = false;
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
          : "") +
        // 单元文件预览：设计稿调度页底部就是这块深色代码块。
        // 后端 program_unit_preview 直接调 us-service 的 render，和真正写盘的是同一份，
        // 不是前端拼出来糊人的。
        `<div class="sec unit-block" id="wb-unit" style="margin-bottom:0">` +
        `<div class="unit-head"><span class="utitle">…</span><span class="upath"></span></div>` +
        `<div class="unit-empty">…</div></div>`;
      box.querySelectorAll("#wb-seg button").forEach((b) => {
        b.onclick = () => setProgramSupervisor(it, b.dataset.sup === "native");
      });
      const save = $("#wb-save-sched");
      if (save) save.onclick = () => saveProgramSched(it);
      loadUnitPreview(it.id);
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

  // 单元文件预览。与真正写盘用的是同一套渲染（后端 us-service 的 plist/unit/xml），
  // 所以这里看到的就是装进系统调度器的那一份。
  // 竞态处理：连续点几行时只认最后一次请求的结果，否则会把 A 的单元画到 B 上。
  let unitSeq = 0;
  async function loadUnitPreview(id) {
    const box = document.getElementById("wb-unit");
    if (!box) return;
    const seq = ++unitSeq;
    try {
      const r = await invoke("program_unit_preview", { programId: id });
      if (seq !== unitSeq) return;
      const cur = document.getElementById("wb-unit");
      if (!cur) return;
      const label = r.platform === "schtasks" ? "Task Scheduler XML" : r.platform;
      const title = cur.querySelector(".utitle");
      const upath = cur.querySelector(".upath");
      if (title) title.textContent = t("wb.sched_unit", { p: label });
      if (upath) {
        upath.textContent = r.path || "";
        upath.title = r.path || "";
      }
      cur.innerHTML =
        `<div class="unit-head"><span class="utitle">${esc(t("wb.sched_unit", { p: label }))}</span>` +
        `<span class="upath" title="${esc(r.path || "")}">${esc(r.path || "")}</span></div>` +
        `<pre class="unit-pre">${esc(r.content || "")}</pre>`;
    } catch (e) {
      if (seq !== unitSeq) return;
      const cur = document.getElementById("wb-unit");
      if (!cur) return;
      cur.innerHTML =
        `<div class="unit-head"><span class="utitle">${esc(t("wb.tab_sched"))}</span></div>` +
        `<div class="unit-empty">${esc(t("wb.sched_unit_fail", { err: String(e) }))}</div>`;
    }
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
    renderInspHead(it);
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
    // updateUpdateCard 在 strip 隐藏时会直接 return，上面那次调用可能什么都没写。
    // strip 可见之后必须再刷一次，否则版本行和「操作」行停在旧值上。
    if (typeof updateUpdateCard === "function") updateUpdateCard();
  }

  // ---------- 概览页 ----------

  // 内存曲线 + 事件时间线：跟着选中项走。晚到的响应如果已经不是当前选中项就丢掉，
  // 否则快速点两行时会把上一行的曲线画到这一行上。
  async function loadInsight(id) {
    try {
      const r = await invoke("program_insight", { programId: id });
      if (!st.selected || st.selected.id !== id) return;
      st.insight = r || null;
      st.insightFor = id;
      if (st.tab === "ov") renderOverview();
    } catch (e) {
      /* 取不到就保持原样，别把界面卡在 loading */
    }
  }

  // 内存曲线：把采样点画成一条折线。少于两个点不画——一条直线没有信息量，
  // 反而让人以为「内存一直没变」。
  function sparkHTML(hist) {
    if (!Array.isArray(hist) || hist.length < 2) return "";
    const kb = hist.map((s) => Number(s.rss_kb != null ? s.rss_kb : s.kb) || 0);
    const max = Math.max(...kb);
    const min = Math.min(...kb);
    // 内存几乎不动时（比如一个 sleep），差值归一化会把线压到最底下贴着边框，
    // 看着像一条下划线。这种情况直接画在中间，表达「平稳」而不是「贴底」。
    const flat = max - min < Math.max(1, max * 0.02);
    const n = kb.length;
    const pts = kb
      .map((v, i) => {
        const x = ((i / (n - 1)) * 100).toFixed(2);
        const y = flat ? 9 : 17 - ((v - min) / Math.max(max - min, 1)) * 15;
        return `${x},${y.toFixed(2)}`;
      })
      .join(" ");
    return (
      `<svg class="spark ram" viewBox="0 0 100 18" preserveAspectRatio="none" aria-hidden="true">` +
      `<polyline points="${pts}"/></svg>`
    );
  }

  // KB → 数字 + 单位。小于 1MB 就按 KB 显示，别把 300KB 写成 0.3MB。
  function fmtMem(kb) {
    if (!kb) return { v: "—", unit: "" };
    if (kb < 1024) return { v: String(kb), unit: "KB" };
    return { v: kb < 10240 ? (kb / 1024).toFixed(1) : String(Math.round(kb / 1024)), unit: "MB" };
  }

  // 事件文案按 kind 现拼（后端只存 kind / pid / code，存了中文就切不了语言）
  function eventText(ev) {
    switch (ev.kind) {
      case "start":
        return ev.pid ? t("ev.start", { pid: ev.pid }) : t("ev.start_nopid");
      case "stop":
        return t("ev.stop");
      case "exit":
        return ev.code == null ? t("ev.exit_nocode") : t("ev.exit", { code: ev.code });
      case "install":
        return t("ev.install");
      case "autostart_on":
        return t("ev.autostart_on");
      case "autostart_off":
        return t("ev.autostart_off");
      case "sup_native":
        return t("ev.sup_native");
      case "sup_managed":
        return t("ev.sup_managed");
      default:
        return esc(ev.kind);
    }
  }

  function hhmm(at) {
    const d = new Date((at || 0) * 1000);
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  }

  // 导入时间是跨月跨年的历史信息，只有时分秒不够用
  function ymdhm(at) {
    const d = new Date((at || 0) * 1000);
    if (isNaN(d.getTime()) || !at) return "—";
    const p = (n) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  // 概览按 D 方案的五块排：KPI → 运行配置 → 依赖与关联 → 最近事件 → 日志。
  // 数字全部来自后端（uptime / RSS / 事件表），没有一处是编的。
  function renderOverview() {
    const box = $("#pane-ov");
    if (!box) return;
    const it = st.selected;
    if (!it) {
      box.innerHTML = `<div class="d-empty">${esc(t("wb.select_hint"))}</div>`;
      return;
    }
    const p = typeof programs !== "undefined" && programs ? programs.find((x) => x.id === it.id) : null;
    const stt =
      typeof statuses !== "undefined" && statuses ? (statuses.find((x) => x.id === it.id) || {}).status || null : null;
    const ins = st.insightFor === it.id ? st.insight : null;

    const port = it.port || (stt && stt.ready_port) || null;
    const hasVer = it.version && it.version !== "-";
    const ver = hasVer ? "v" + it.version : "—";
    const upd = it.has_update && it.latest_version;
    const running = it.state === "running";
    const mem = fmtMem(it.mem_kb || (ins && ins.mem_kb));
    const sup =
      it.kind === "service"
        ? t("wb.manual_service")
        : it.supervisor === "native"
          ? t("wb.sup_native")
          : t("wb.sup_managed");

    // 程序：用 shell.json 里的 args / binary / repo；服务：用命令与工作目录
    const rows = [];
    if (it.kind === "service") {
      rows.push([t("wb.f_cmd"), it.command || "—"]);
      rows.push([t("wb.f_args"), (it.args || []).join(" ") || "—"]);
      rows.push([t("wb.ov_cwd"), it.cwd || "—"]);
      rows.push([t("wb.ov_sup_row"), sup]);
      rows.push([t("wb.ov_auto"), it.autostart ? t("wb.ov_on") : t("wb.ov_off")]);
    } else {
      rows.push([t("wb.ov_bin"), (stt && stt.bin_path) || (p && p.binary) || "—"]);
      rows.push([t("wb.ov_args"), (p && (p.args || []).join(" ")) || "—"]);
      rows.push([t("wb.ov_sup_row"), sup]);
      rows.push([t("wb.ov_auto"), it.autostart ? t("wb.ov_on") : t("wb.ov_off")]);
      rows.push([t("wb.ov_cat"), it.category || "—"]);
      rows.push([t("wb.ov_repo"), (p && p.repo) || "—"]);
    }

    // 依赖与关联：只列真的存在的东西。没有依赖就说没有，不摆两个假 chip。
    const deps = Array.isArray(it.depends_on) ? it.depends_on.filter(Boolean) : [];
    const depChips = deps.length
      ? deps.map((d) => `<span class="chip"><b>depends</b>: ${esc(d)}</span>`).join("")
      : `<span class="chip">${esc(t("wb.ov_dep_none"))}</span>`;
    const extraChips =
      it.kind === "service"
        ? [
            it.ready_port ? `<span class="chip"><b>ready</b>: :${esc(it.ready_port)}</span>` : "",
            it.ready_file ? `<span class="chip"><b>ready</b>: ${esc(it.ready_file)}</span>` : "",
            it.scope ? `<span class="chip"><b>scope</b>: ${esc(it.scope)}</span>` : "",
          ]
            .filter(Boolean)
            .join("")
        : `<span class="chip"><b>${esc(t("wb.ov_tmpl"))}</b> ${esc(it.id)}</span>`;

    // 导入信息：这个程序从哪来、什么时候来的。设计稿里这块本来就有，
    // 后端 workspace_list 已经带了 imported_at / template_source / repo 三个字段。
    // 本地程序没有导入时间（手写的），就不硬造一行「—」。
    // template_source 可能是整条 raw.githubusercontent.com URL，原样铺开会占三行，
    // 只留末尾两段路径（完整值放 title，鼠标悬停能看全）。
    const shortSrc = (s) => {
      if (!s || !/^https?:\/\//i.test(s)) return s;
      try {
        const parts = new URL(s).pathname.split("/").filter(Boolean);
        return "…/" + parts.slice(-2).join("/");
      } catch {
        return s;
      }
    };
    const importRows = [];
    if (it.kind !== "service") {
      if (it.imported_at) importRows.push([t("wb.ov_imported_at"), ymdhm(it.imported_at), ""]);
      if (it.template_source)
        importRows.push([t("wb.ov_source"), shortSrc(it.template_source), it.template_source]);
      if (p && p.repo) importRows.push([t("wb.ov_repo"), p.repo, ""]);
    }
    const importHTML = importRows.length
      ? `<dl class="kv kv-sub">${importRows
          .map(
            ([k, v, full]) =>
              `<dt>${esc(k)}</dt><dd${full ? ` title="${esc(full)}"` : ""}>${esc(v)}</dd>`
          )
          .join("")}</dl>`
      : "";

    // 最近事件：后端 ring buffer，跨重启保留（insight.json）
    const events = (ins && ins.events) || [];
    const evHTML = events.length
      ? `<ul class="tl">${events
          .slice()
          .reverse()
          .map((ev) => `<li><span class="t">${hhmm(ev.at)}</span><span class="d">${eventText(ev)}</span></li>`)
          .join("")}</ul>`
      : `<div class="d-muted">${esc(t("wb.ov_no_events"))}</div>`;

    // KPI 顺序对齐设计稿：运行时长 → 内存 → 版本 → 监听。
    // 「现在活不活」比「绑在哪」更该先看到，所以端口放最后。
    const kpiUptime =
      `<div class="kpi"><div class="k">${esc(t("wb.ov_uptime"))}</div>` +
      // 外部接管：壳没有句柄，拿不到启动时刻，显示 PID 比显示「—」有用得多
      `<div class="v" style="font-size:15px">${esc(
        it.external_pid ? "PID " + it.external_pid : fmtUptime(it.uptime)
      )}</div>` +
      `<div class="kh${it.external_pid ? " up" : ""}">${esc(
        it.external_pid ? t("wb.state_external") : running ? t("wb.ov_running") : t("wb.ov_stopped")
      )}</div></div>`;
    const kpiMem =
      `<div class="kpi"><div class="k">${esc(t("wb.ov_mem"))}</div>` +
      `<div class="v">${esc(mem.v)}${mem.unit ? `<small>${esc(mem.unit)}</small>` : ""}</div>` +
      sparkHTML(ins && ins.mem_hist) +
      `<div class="kh">${esc(running ? t("wb.ov_mem_live") : t("wb.ov_mem_idle"))}</div></div>`;
    const kpiVer =
      `<div class="kpi"><div class="k">${esc(t("wb.ov_ver"))}</div><div class="v" style="font-size:15px">${esc(ver)}</div>` +
      `<div class="kh${upd ? " up" : ""}">${
        !hasVer ? "—" : upd ? esc(t("wb.ov_upd_to", { v: it.latest_version })) : esc(t("wb.ov_latest"))
      }</div></div>`;
    const kpiPort =
      `<div class="kpi"><div class="k">${esc(t("wb.ov_port"))}</div><div class="v">${port ? ":" + port : "—"}${
        port ? "<small>TCP</small>" : ""
      }</div><div class="kh">${port ? "127.0.0.1" : esc(t("wb.ov_nobind"))}</div></div>`;

    box.innerHTML =
      `<div class="kpis">${kpiUptime}${kpiMem}${kpiVer}${kpiPort}</div>` +
      `<div class="sec"><div class="sec-h">${esc(t("wb.ov_cfg"))}<span class="spacer"></span>` +
      `<span class="hint">${esc(t("wb.ov_snapshot"))}</span></div><div class="sec-b"><dl class="kv">` +
      rows.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join("") +
      `</dl></div></div>` +
      `<div class="sec"><div class="sec-h">${esc(t("wb.ov_dep"))}<span class="spacer"></span>` +
      `<span class="hint">${esc(
        it.kind === "service"
          ? t("wb.ov_dep_hint_svc")
          : it.supervisor === "native"
            ? t("wb.ov_dep_hint")
            : t("wb.ov_dep_hint_managed"),
      )}</span></div>` +
      `<div class="sec-b"><div class="chips">${depChips}${extraChips}</div>${importHTML}</div></div>` +
      `<div class="sec"><div class="sec-h">${esc(t("wb.ov_events"))}<span class="spacer"></span>` +
      `<span class="hint">${events.length}</span></div><div class="sec-b">${evHTML}</div></div>` +
      `<div class="sec ov-log" style="margin-bottom:0">` +
      `<div class="manage-log" data-open="1">` +
      // 标题给文件名而不是「程序日志」：出问题时要能立刻知道该 tail 哪个文件
      `<div class="manage-log-head"><span class="manage-log-title">${esc(it.id)}.log</span>` +
      `<span class="spacer"></span>` +
      `<button class="log-action" data-ovlog-follow="1">${esc(t("wb.ov_log_follow"))}</button>` +
      `<button class="log-action" data-ovlog="1">${esc(t("wb.ov_log_expand"))}</button></div>` +
      `<pre class="term-body ov-term"></pre></div></div>`;

    // 「跟随」：滚动到底 + 之后每次刷新都自动贴底。展开则跳到日志页看全量。
    const followBtn = box.querySelector("[data-ovlog-follow]");
    if (followBtn) {
      followBtn.onclick = () => {
        const t2 = box.querySelector(".ov-term");
        if (t2) t2.scrollTop = t2.scrollHeight;
        if (typeof refreshManageLog === "function") refreshManageLog();
      };
    }

    // 日志卡复用 main.js 的尾随缓存，保证和「日志」页看到的是同一份内容
    const term = box.querySelector(".ov-term");
    const tail = typeof window.usLogTail === "function" ? window.usLogTail(6) : [];
    if (term) {
      if (!tail.length) {
        term.textContent = t("wb.ov_log_empty");
        term.classList.add("ov-term-empty");
      } else {
        const frag = document.createDocumentFragment();
        for (const line of tail) frag.appendChild(window.usLogLine ? window.usLogLine(line) : textNode(line));
        term.appendChild(frag);
      }
    }
  }

  function textNode(s) {
    const d = document.createElement("div");
    d.textContent = s;
    return d;
  }

  // 头像 + 标题 + 副标题：点行后立刻刷，不等 main.js 的 switchCurrent。
  // switchCurrent 里要 await get_values / 版本检查，慢的时候标题会滞后好几秒，
  // 期间头像已经换了、标题还是上一个程序，看着像点错了行。
  // main.js 的 renderHeader() 也转调这里：头部只能有一个写入方，
  // 两边各写一半的结果是副标题被冲空、repo 链接插两遍。
  function renderInspHead(it) {
    const ava = document.getElementById("d-avatar");
    const title = document.getElementById("prog-title");
    const sub = document.getElementById("prog-sub");
    const chips = document.getElementById("status-chips");
    if (!it) {
      if (ava) {
        ava.textContent = "·";
        ava.style.background = "var(--panel-2)";
        ava.style.color = "var(--muted)";
      }
      return;
    }
    // main.js 传进来的是 Program（没有 category / state / repo），用列表项补全；
    // 列表项才是 workspace_list 的原始结构，字段最全。
    const row = st.items.find((x) => x.id === it.id) || it;
    if (ava) {
      ava.textContent = (row.name || "?").slice(0, 1).toUpperCase();
      ava.style.background = hueOf(row.id);
      ava.style.color = "#fff";
    }
    if (title) title.textContent = row.name || row.id;
    if (sub) {
      const kind = row.kind === "service" ? t("wb.manual_service") : row.category;
      sub.textContent = [row.id, kind].filter(Boolean).join(" · ");
      const repo = (row.repo || "").trim();
      if (repo) {
        const a = document.createElement("a");
        a.className = "repo-link";
        a.textContent = "GitHub";
        a.title = `GitHub · ${repo}`;
        a.href = "#";
        a.onclick = (e) => {
          e.preventDefault();
          if (typeof openExternal === "function") openExternal("https://github.com/" + repo);
        };
        sub.appendChild(document.createTextNode(" · "));
        sub.appendChild(a);
      }
    }
    if (chips) {
      chips.innerHTML = row.external_pid
        ? `<span class="pill ext" title="${esc(t("wb.ext_tip", { pid: row.external_pid }))}">${esc(t("wb.state_external"))}</span>`
        : `<span class="pill${row.state === "running" ? " ok" : ""}">${esc(stateText(row.state))}</span>`;
    }
  }

  // ---------- tab ----------

  function showTab(name) {
    st.tab = name;
    document.querySelectorAll("#wb-tabs .wb-tab").forEach((b) => {
      b.classList.toggle("on", b.dataset.wbtab === name);
    });
    const panes = {
      ov: $("#pane-ov"),
      cfg: $("#pane-cfg"),
      sched: $("#pane-sched"),
      log: $("#pane-log"),
      upd: $("#pane-upd"),
    };
    for (const [k, node] of Object.entries(panes)) {
      if (node) node.hidden = k !== name;
    }
    if (name === "ov") {
      // 概览里的曲线/事件是单独取的，切回来时补一次，避免显示上一次的残留
      if (st.selected && st.insightFor !== st.selected.id) loadInsight(st.selected.id);
      renderOverview();
    }
    // 切到日志页：日志窗刚被显示出来，补一次尾随（隐藏时高度为 0，滚动位置是旧的）
    if (name === "log" && typeof refreshManageLog === "function") refreshManageLog();
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
        const lines = r.lines || [];
        const pre = document.getElementById("manage-log-content");
        if (pre) {
          // 跟程序日志共用同一套行渲染，时间戳/级别一样染色
          pre.innerHTML = "";
          const frag = document.createDocumentFragment();
          for (const line of lines) frag.appendChild(window.usLogLine ? window.usLogLine(line) : textNode(line));
          pre.appendChild(frag);
          pre.scrollTop = pre.scrollHeight;
        }
        // 服务日志是「旧→新」铺的，缓存按「新→旧」交回去，概览卡才取到最新几行
        if (typeof window.usSetLogTail === "function") window.usSetLogTail(lines.slice().reverse());
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
      // 刷新后对象身份变了，概览页读的是 st.selected，得跟着重画一次
      if (st.selected) {
        renderInspHead(st.selected);
        if (st.tab === "ov") renderOverview();
        // 曲线要跟着采样走：每轮轮询补一次 insight，5 秒一个点才动得起来
        loadInsight(st.selected.id);
      }
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

  // 启动/重启必须带上已保存的字段值，否则模板里的 {port} / {token} 这类占位符
  // 会被渲染成空串（裸调 RPC 传 values: {} 就是这样，见 2026-11-23 的踩坑记录）。
  async function itemValues(it) {
    try {
      return (await invoke("get_values", { programId: it.id })) || {};
    } catch (e) {
      return {};
    }
  }

  // 表格「操作」列：打开网站 / 启停 / 重启 / 更多
  async function quickAct(it, act) {
    if (act === "site") {
      const p = typeof programs !== "undefined" && programs ? programs.find((x) => x.id === it.id) : null;
      if (p && typeof openSiteOf === "function") openSiteOf(p);
      return;
    }
    if (act === "more") {
      const p = typeof programs !== "undefined" && programs ? programs.find((x) => x.id === it.id) : null;
      if (p && typeof ctxMenuAt === "function" && typeof progItemMenu === "function") {
        const btn = document.querySelector(`[data-act="more"][data-id="${CSS.escape(it.id)}"]`);
        const r = btn ? btn.getBoundingClientRect() : { left: 0, bottom: 0 };
        ctxMenuAt(r.left, r.bottom + 4, progItemMenu(p));
      }
      return;
    }
    try {
      if (it.kind === "service") {
        await invoke("services_action", { id: it.id, action: act });
      } else if (act === "start") {
        await invoke("start_program", { programId: it.id, values: await itemValues(it) });
      } else if (act === "stop") {
        await invoke("stop_program", { programId: it.id });
      } else if (act === "restart") {
        await invoke("restart_program", { programId: it.id, values: await itemValues(it) });
      }
      // 走 reloadPrograms 一并刷新 main.js 的 programs/statuses，否则检查器还拿着旧状态
      if (typeof reloadPrograms === "function") await reloadPrograms();
      else await load();
    } catch (e) {
      showNotice(String(e), true);
    }
  }

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
      // 行内操作列：必须在「点行选中」之前拦掉，否则会连带把详情切过去
      const ab = e.target.closest("[data-act]");
      if (ab) {
        e.stopPropagation();
        const it = st.items.find((x) => x.id === ab.dataset.id);
        if (it) quickAct(it, ab.dataset.act);
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
      if (tr) {
        select(tr.dataset.id, true);
        return;
      }
      // 点表格空白（表头、表体下方留白）等于「点空白处」：收起详情。
      closeInspector();
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
    // 点表格区域的空白（表头下方留白、表体之后的空档）关详情。
    // 绑在 wrap 而不是 table 上：table 之外、wrap 之内的 padding 也是「空白处」。
    const wrap = document.getElementById("wb-tablewrap");
    if (wrap) {
      wrap.addEventListener("click", (e) => {
        // 行、排序表头、行内按钮/开关各有自己的 handler，不在这里插手
        if (e.target.closest("tr[data-id]") || e.target.closest("th[data-sort]")) return;
        if (e.target.closest("button, input, select, label, a")) return;
        closeInspector();
      });
    }
    const close = document.getElementById("wb-insp-close");
    if (close) close.onclick = () => closeInspector();
    // 遮罩就是窄屏抽屉下的「空白处」：点它关详情
    const mask = document.getElementById("wb-insp-mask");
    if (mask) mask.onclick = () => closeInspector();
    // Esc 同样能关。弹窗自己会吃掉 Esc（有 open 的 modal 时不处理），
    // 免得「关弹窗」顺手把详情也关了。
    document.addEventListener("keydown", (e) => {
      if (e.key !== "Escape") return;
      if (document.querySelector(".modal:not([hidden])")) return;
      closeInspector();
    });
  }

  // 收起详情：窄屏摘抽屉 class，宽屏摘常驻面板（wb-insp-closed 只在 ≥1101 生效）。
  // 两个都摘，是因为同一份状态要在两种布局下都正确。
  function closeInspector() {
    document.body.classList.remove("wb-insp-open");
    document.body.classList.add("wb-insp-closed");
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
    // 概览页日志卡上的「展开」：概览是每次重绘的，只能挂在容器上做事件委托
    const ov = document.getElementById("pane-ov");
    if (ov) {
      ov.addEventListener("click", (e) => {
        if (e.target.closest("[data-ovlog]")) showTab("log");
      });
    }
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
    renderInspHead,
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
