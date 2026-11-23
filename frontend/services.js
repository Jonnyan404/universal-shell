// 服务看板（主界面「服务」页）。
// 只依赖 api.js 暴露的 invoke；渲染、动作、日志抽屉都在本文件内自洽。
// 服务由系统调度器负责拉起，这里关了不影响它们运行。

// 文案在 shared/locales（键前缀 svc.），由 build.rs 生成到 frontend/locales。
// 这里只做前缀补全：t() 由 main.js 提供，语言切换时重建本视图即可。
const L = (k, vars) => t("svc." + k, vars);
const esc = (s) =>
  String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

const svcState = { data: { services: [] }, openId: null, timer: null, editing: null, auto: true };

function svcRoot() {
  return document.querySelector("#services-view");
}

function svcRender() {
  const root = svcRoot();
  if (!root || root.hidden) return;
  const s = svcState.data.services || [];
  const running = s.filter((x) => x.state === "running").length;
  const enabled = s.filter((x) => x.state !== "not_installed").length;

  root.innerHTML = `
    <div class="panel svc-panel">
      <div class="svc-head">
        <div>
          <div class="svc-title">${esc(L("title"))}</div>
          <div class="svc-sub">${esc(svcState.data.platform || "")} · ${esc(svcState.data.config || "")}</div>
        </div>
        <span class="spacer"></span>
        <div class="pills">
          <span class="pill">${esc(L("summary", { total: s.length, running }))}</span>
        </div>
        <label class="svc-toggle"><input type="checkbox" id="svc-auto" ${svcState.auto ? "checked" : ""}/>${esc(L("auto"))}</label>
        <button id="svc-new">${esc(L("add"))}</button>
        <button id="svc-refresh">${esc(L("refresh"))}</button>
      </div>
      <div class="svc-grid">${s.length ? s.map(svcCard).join("") : `<div class="svc-empty">${esc(L("empty"))}</div>`}</div>
    </div>`;

  root.querySelector("#svc-refresh").onclick = () => svcLoad();
  root.querySelector("#svc-new").onclick = () => svcEdit(null);
  root.querySelector("#svc-auto").onchange = (e) => {
    svcState.auto = e.target.checked;
    if (svcState.auto) svcLoad();
  };
  root.querySelectorAll("button[data-svc-act]").forEach((b) => {
    b.onclick = () => svcAction(b.dataset.svcAct, b.dataset.id, b);
  });
}

function svcCard(x) {
  const cmd = [x.command, ...(x.args || [])].join(" ");
  const pillCls = x.state === "running" ? "ok" : x.state === "unknown" ? "warn" : "";
  const installed = x.state !== "not_installed";
  const canRun = installed && x.enabled;
  const badges = [
    `<span class="svc-badge">${esc(x.scope === "system" ? L("scope.system") : L("scope.user"))}</span>`,
    `<span class="svc-badge">${esc(
      x.restart === "always" ? L("restart.always") : x.restart === "on_failure" ? L("restart.on_failure") : L("restart.never")
    )}</span>`,
    x.delay_secs ? `<span class="svc-badge">${x.delay_secs}s</span>` : "",
    (x.depends_on || []).length
      ? `<span class="svc-badge svc-badge-dep">${esc(L("wait_for"))} ${esc((x.depends_on || []).join(" "))}</span>`
      : "",
    x.ready_port ? `<span class="svc-badge">:${x.ready_port}</span>` : "",
    x.enabled ? "" : `<span class="svc-badge">off</span>`,
    x.origin
      ? `<span class="svc-badge svc-badge-origin">${esc(
          L("origin", { id: String(x.origin).replace("program:", "") })
        )}</span>`
      : "",
  ].join("");
  return `<div class="svc-card ${x.enabled ? "" : "svc-off"}">
    <div class="svc-row">
      <span class="svc-dot svc-dot-${x.state}"></span>
      <span class="svc-name">${esc(x.name)}</span>
      <span class="svc-id">${esc(x.id)}</span>
      <span class="pill ${pillCls}">${esc(L("state." + x.state))}</span>
    </div>
    <div class="svc-cmd" title="${esc(cmd)}">${esc(x.command)} <span>${esc((x.args || []).join(" "))}</span></div>
    <div class="svc-badges">${badges}</div>
    <div class="svc-last" title="${esc(x.last_line || "")}">${x.last_line ? esc(x.last_line) : esc(L("nolog"))}</div>
    <div class="svc-actions">
      <button data-svc-act="start" data-id="${x.id}" ${canRun && x.state !== "running" ? "" : "disabled"}>${esc(L("start"))}</button>
      <button data-svc-act="restart" data-id="${x.id}" ${canRun ? "" : "disabled"}>${esc(L("restart"))}</button>
      <button data-svc-act="stop" data-id="${x.id}" ${x.state === "running" ? "" : "disabled"}>${esc(L("stop"))}</button>
      <button data-svc-act="log" data-id="${x.id}">${esc(L("log"))}</button>
      <button data-svc-act="edit" data-id="${x.id}">${esc(L("edit"))}</button>
      <span class="spacer"></span>
      ${
        installed
          ? `<button class="op-danger" data-svc-act="disable" data-id="${x.id}">${esc(L("disable"))}</button>`
          : `<button data-svc-act="enable" data-id="${x.id}" ${x.enabled ? "" : "disabled"}>${esc(L("enable"))}</button>`
      }
    </div>
  </div>`;
}

async function svcLoad() {
  try {
    svcState.data = await invoke("services_list");
    svcRender();
  } catch (e) {
    if (window.showNotice) showNotice(String(e), true);
  }
}

async function svcAction(act, id, btn) {
  if (act === "log") return svcOpenLog(id);
  if (act === "edit") return svcEdit(id);
  btn.disabled = true;
  try {
    const r = await invoke("services_action", { id, action: act });
    if (window.showNotice) showNotice(r.message || act);
    await svcLoad();
  } catch (e) {
    if (window.showNotice) showNotice(String(e), true);
  } finally {
    btn.disabled = false;
  }
}

// ---------- 日志抽屉 ----------

function svcDrawer() {
  let d = document.querySelector("#svc-drawer");
  if (d) return d;
  d = document.createElement("aside");
  d.id = "svc-drawer";
  d.className = "svc-drawer";
  d.innerHTML = `
    <div class="svc-drawer-head">
      <div style="min-width:0">
        <div class="svc-title" id="svc-d-title"></div>
        <div class="svc-sub" id="svc-d-path"></div>
      </div>
      <span class="spacer"></span>
      <button id="svc-d-close">${esc(L("close"))}</button>
    </div>
    <div class="svc-drawer-bar">
      <input type="search" id="svc-d-search" placeholder="${esc(L("search_ph"))}"/>
      <label class="svc-toggle"><input type="checkbox" id="svc-d-follow" checked/>${esc(L("follow"))}</label>
    </div>
    <pre class="term-body svc-drawer-log" id="svc-d-log"></pre>
    <div class="svc-drawer-foot"><span id="svc-d-meta"></span></div>`;
  document.body.appendChild(d);
  d.querySelector("#svc-d-close").onclick = svcCloseLog;
  d.querySelector("#svc-d-search").oninput = () => svcPullLog();
  return d;
}

async function svcOpenLog(id) {
  const svc = (svcState.data.services || []).find((x) => x.id === id);
  const d = svcDrawer();
  svcState.openId = id;
  d.querySelector("#svc-d-title").textContent = svc ? svc.name : id;
  d.querySelector("#svc-d-path").textContent = svc ? svc.log : "";
  d.classList.add("open");
  await svcPullLog();
  clearInterval(svcState.timer);
  svcState.timer = setInterval(svcPullLog, 1500);
}

function svcCloseLog() {
  svcState.openId = null;
  clearInterval(svcState.timer);
  const d = document.querySelector("#svc-drawer");
  if (d) d.classList.remove("open");
}

async function svcPullLog() {
  if (!svcState.openId) return;
  const d = svcDrawer();
  const pre = d.querySelector("#svc-d-log");
  try {
    const r = await invoke("services_log", { id: svcState.openId, tail: 400 });
    const q = d.querySelector("#svc-d-search").value.trim();
    const lines = r.lines || [];
    pre.innerHTML = q
      ? lines.map((l) => esc(l).split(esc(q)).join(`<span class="svc-hit">${esc(q)}</span>`)).join("\n")
      : esc(lines.join("\n"));
    d.querySelector("#svc-d-meta").textContent = `${lines.length} · ${(r.size / 1024).toFixed(1)} KB`;
    if (d.querySelector("#svc-d-follow").checked) pre.scrollTop = pre.scrollHeight;
  } catch (e) {
    pre.textContent = String(e);
  }
}

// ---------- 新建 / 编辑 ----------

function svcEditor() {
  let m = document.querySelector("#svc-editor");
  if (m) return m;
  m = document.createElement("div");
  m.id = "svc-editor";
  m.className = "modal";
  m.hidden = true;
  m.innerHTML = `
    <div class="modal-box">
      <div class="modal-head"><span id="svc-e-title"></span><span class="spacer"></span>
        <button class="modal-close" id="svc-e-close">✕</button></div>
      <div class="edit-scroll">
        <label class="edit-label">${esc(L("f.id"))}</label><input id="svc-e-id" type="text" placeholder="dufs"/>
        <label class="edit-label">${esc(L("f.name"))}</label><input id="svc-e-name" type="text"/>
        <label class="edit-label">${esc(L("f.command"))}</label><input id="svc-e-command" type="text" placeholder="/usr/local/bin/dufs"/>
        <label class="edit-label">${esc(L("f.args"))}</label><textarea id="svc-e-args" rows="3" class="edit-textarea"></textarea>
        <label class="edit-label">${esc(L("f.env"))}</label><textarea id="svc-e-env" rows="2" class="edit-textarea" placeholder="RUST_LOG=info"></textarea>
        <label class="edit-label">${esc(L("f.cwd"))}</label><input id="svc-e-cwd" type="text"/>
        <label class="edit-label">${esc(L("f.log"))}</label><input id="svc-e-log" type="text"/>
        <div class="svc-two">
          <div><label class="edit-label">${esc(L("f.delay"))}</label><input id="svc-e-delay" type="number" min="0" value="0"/></div>
          <div><label class="edit-label">${esc(L("f.restart"))}</label>
            <select id="svc-e-restart">
              <option value="never">${esc(L("restart.never"))}</option>
              <option value="on_failure">${esc(L("restart.on_failure"))}</option>
              <option value="always">${esc(L("restart.always"))}</option>
            </select></div>
          <div><label class="edit-label">${esc(L("f.scope"))}</label>
            <select id="svc-e-scope">
              <option value="user">${esc(L("scope.user"))}</option>
              <option value="system">${esc(L("scope.system"))}</option>
            </select></div>
        </div>
        <label class="edit-label">${esc(L("f.deps"))}</label>
        <input id="svc-e-deps" type="text" placeholder="db, cache"/>
        <div class="svc-hint">${esc(L("f.deps_hint"))}</div>
        <div class="svc-two">
          <div><label class="edit-label">${esc(L("f.wait"))}</label><input id="svc-e-wait" type="number" min="0" value="60"/></div>
          <div><label class="edit-label">${esc(L("f.rport"))}</label><input id="svc-e-rport" type="text" placeholder="5000"/></div>
          <div><label class="edit-label">${esc(L("f.rfile"))}</label><input id="svc-e-rfile" type="text"/></div>
        </div>
        <div class="svc-checks">
          <label><input type="checkbox" id="svc-e-enabled" checked/>${esc(L("f.enabled"))}</label>
          <label><input type="checkbox" id="svc-e-apply" checked/>${esc(L("f.apply"))}</label>
        </div>
        <div class="edit-footer">
          <button id="svc-e-del" class="op-danger" hidden>${esc(L("del"))}</button>
          <span class="spacer"></span>
          <button id="svc-e-cancel">${esc(L("cancel"))}</button>
          <button id="svc-e-save" class="edit-save">${esc(L("save"))}</button>
        </div>
      </div>
    </div>`;
  document.body.appendChild(m);
  m.querySelector("#svc-e-close").onclick = svcCloseEditor;
  m.querySelector("#svc-e-cancel").onclick = svcCloseEditor;
  m.querySelector("#svc-e-save").onclick = svcSave;
  m.querySelector("#svc-e-del").onclick = svcDelete;
  return m;
}

function svcEdit(id) {
  const svc = id ? (svcState.data.services || []).find((x) => x.id === id) : null;
  const m = svcEditor();
  svcState.editing = svc;
  m.hidden = false;
  m.querySelector("#svc-e-title").textContent = svc ? L("editing") : L("creating");
  m.querySelector("#svc-e-id").value = svc ? svc.id : "";
  m.querySelector("#svc-e-id").disabled = !!svc;
  m.querySelector("#svc-e-name").value = svc ? svc.name : "";
  m.querySelector("#svc-e-command").value = svc ? svc.command : "";
  m.querySelector("#svc-e-args").value = svc ? (svc.args || []).join("\n") : "";
  m.querySelector("#svc-e-env").value = svc
    ? Object.entries(svc.env || {}).map(([k, v]) => k + "=" + v).join("\n")
    : "";
  m.querySelector("#svc-e-cwd").value = svc ? svc.cwd || "" : "";
  m.querySelector("#svc-e-log").value = svc ? svc.log : "";
  m.querySelector("#svc-e-delay").value = svc ? svc.delay_secs : 0;
  m.querySelector("#svc-e-deps").value = svc ? (svc.depends_on || []).join(", ") : "";
  m.querySelector("#svc-e-wait").value = svc ? svc.wait_timeout_secs || 60 : 60;
  m.querySelector("#svc-e-rport").value = svc && svc.ready_port ? svc.ready_port : "";
  m.querySelector("#svc-e-rfile").value = svc ? svc.ready_file || "" : "";
  m.querySelector("#svc-e-restart").value = svc ? svc.restart : "never";
  m.querySelector("#svc-e-scope").value = svc ? svc.scope : "user";
  m.querySelector("#svc-e-enabled").checked = svc ? svc.enabled : true;
  m.querySelector("#svc-e-apply").checked = !!svc;
  m.querySelector("#svc-e-del").hidden = !svc;
}

function svcCloseEditor() {
  const m = document.querySelector("#svc-editor");
  if (m) m.hidden = true;
  svcState.editing = null;
}

async function svcSave() {
  const m = svcEditor();
  const editing = svcState.editing;
  const logVal = m.querySelector("#svc-e-log").value.trim();
  const payload = {
    id: m.querySelector("#svc-e-id").value.trim(),
    name: m.querySelector("#svc-e-name").value.trim(),
    description: editing ? editing.description || "" : "",
    command: m.querySelector("#svc-e-command").value.trim(),
    args: m.querySelector("#svc-e-args").value,
    env: m.querySelector("#svc-e-env").value,
    cwd: m.querySelector("#svc-e-cwd").value,
    // 未改动就不写进配置，避免把默认路径固化成显式字段
    log: editing && logVal === editing.log ? "" : logVal,
    delay_secs: Number(m.querySelector("#svc-e-delay").value) || 0,
    depends_on: m.querySelector("#svc-e-deps").value,
    wait_timeout_secs: Number(m.querySelector("#svc-e-wait").value) || 0,
    ready_port: m.querySelector("#svc-e-rport").value,
    ready_file: m.querySelector("#svc-e-rfile").value,
    restart: m.querySelector("#svc-e-restart").value,
    scope: m.querySelector("#svc-e-scope").value,
    enabled: m.querySelector("#svc-e-enabled").checked,
    apply: m.querySelector("#svc-e-apply").checked,
  };
  try {
    const r = await invoke("services_save", { payload });
    if (window.showNotice) showNotice(r.message || "saved");
    svcCloseEditor();
    await svcLoad();
  } catch (e) {
    if (window.showNotice) showNotice(String(e), true);
  }
}

async function svcDelete() {
  const editing = svcState.editing;
  if (!editing) return;
  if (!confirm(L("confirm_del", { id: editing.id }))) return;
  try {
    const r = await invoke("services_remove", { id: editing.id, uninstall: true });
    if (window.showNotice) showNotice(r.message || "removed");
    svcCloseEditor();
    await svcLoad();
  } catch (e) {
    if (window.showNotice) showNotice(String(e), true);
  }
}

// ---------- 生命周期 ----------

window.usServices = {
  /// 语言切换后重建懒创建的节点，否则里面的按钮文案会停在旧语言
  onLangChange() {
    document.querySelector("#svc-drawer")?.remove();
    document.querySelector("#svc-editor")?.remove();
    svcState.openId = null;
    svcState.editing = null;
    if (!svcRoot()?.hidden) svcRender();
  },
  onShow() {
    svcLoad();
    clearInterval(svcState.timer);
    svcState.timer = setInterval(() => {
      if (svcState.auto && !svcState.openId) svcLoad();
    }, 4000);
  },
  onHide() {
    clearInterval(svcState.timer);
    svcCloseLog();
  },
};
