//! 面向浏览器的 JSON-RPC 层：`POST /api/rpc`，命令与 app-tauri 后端一一对应
//! （镜像同名命令的参数/返回结构），供同一份 SPA 在浏览器与桌面窗口使用。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use shared::config::{Field, Program};
use shared::ShellManager;

use rust_i18n::t;

/// 共享状态：宿主进程(egui / tauri)把同一份 manager 交进来
pub struct RpcState {
    pub manager: Arc<Mutex<ShellManager>>,
    pub config_path: PathBuf,
}

#[derive(Deserialize)]
pub struct RpcRequest {
    pub cmd: String,
    #[serde(default)]
    pub args: serde_json::Map<String, Value>,
}

#[derive(Serialize)]
pub struct RpcResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// -------- 视图结构（与 app-tauri 后端同名同构） --------

#[derive(Serialize)]
struct FieldView {
    key: String,
    label: String,
    kind: String,
    default: String,
    placeholder: String,
    required: bool,
}

#[derive(Serialize)]
struct EnvView {
    key: String,
    value: String,
    label: String,
}

#[derive(Serialize)]
struct ProgramView {
    id: String,
    name: String,
    description: String,
    repo: String,
    binary: String,
    args: Vec<String>,
    fields: Vec<FieldView>,
    env: Vec<EnvView>,
    hidden: bool,
    http_enabled: bool,
    http_version_url: String,
    http_version_json_path: String,
    http_version_regex: String,
    http_sha256_url: String,
    http_urls: Vec<String>,
}

#[derive(Serialize, Clone)]
struct StatusView {
    installed: bool,
    running: bool,
    autostart: bool,
    local_version: String,
    latest_version: Option<String>,
    latest_published: String,
    latest_checked_at: Option<u64>,
    up_to_date: bool,
    bin_path: String,
}

#[derive(Serialize, Clone)]
struct ProgramStatusView {
    id: String,
    name: String,
    repo: String,
    hidden: bool,
    status: StatusView,
}

#[derive(Serialize)]
struct LocaleView {
    effective: String,
    manual: String,
    available: Vec<String>,
}

#[derive(Serialize)]
struct ProxyView {
    accelerate_prefix: String,
    http_proxy: String,
}

#[derive(Serialize)]
struct LogsView {
    text: String,
}

impl StatusView {
    /// 本地即时渲染阶段：未知最新版本；有版本检查缓存则回填
    fn from_local(
        s: &shared::ProgramStatus,
        bin_path: &PathBuf,
        autostart: bool,
        repo: &str,
        vcheck: &BTreeMap<String, (String, u64)>,
    ) -> Self {
        let cached = vcheck.get(repo);
        let latest_ver = cached.map(|(v, _)| v.clone());
        let checked_at = cached.map(|(_, t)| *t);
        let up_to_date = s.installed
            && s.local_version != "-"
            && latest_ver
                .as_deref()
                .map(|v| !shared::version::is_newer(v, &s.local_version))
                .unwrap_or(false);
        Self {
            installed: s.installed,
            running: s.running,
            autostart,
            local_version: s.local_version.clone(),
            latest_version: latest_ver,
            latest_published: String::new(),
            latest_checked_at: checked_at,
            up_to_date,
            bin_path: bin_path.display().to_string(),
        }
    }
}

fn to_field_view(f: &Field) -> FieldView {
    let (kind, label, default, placeholder) = match &f.kind {
        shared::config::FieldKind::String { label, default, placeholder } => {
            ("string", label.clone(), default.clone(), placeholder.clone())
        }
        shared::config::FieldKind::File { label, default, .. } => {
            ("file", label.clone(), default.clone(), String::new())
        }
        shared::config::FieldKind::Directory { label, default, .. } => {
            ("directory", label.clone(), default.clone(), String::new())
        }
        shared::config::FieldKind::Boolean { label, default } => {
            ("boolean", label.clone(), default.to_string(), String::new())
        }
        shared::config::FieldKind::AutoStart { label, default } => {
            ("autostart", label.clone(), default.to_string(), String::new())
        }
    };
    FieldView { key: f.key.clone(), kind: kind.to_string(), label, default, placeholder, required: f.required }
}

fn to_view(p: &Program) -> ProgramView {
    let (http_enabled, http_version_url, http_version_json_path, http_version_regex, http_sha256_url, http_urls) =
        match p.source.as_ref().filter(|s| s.is_http()) {
            Some(src) => (
                true,
                src.version_url.clone(),
                src.version_json_path.clone(),
                src.version_regex.clone(),
                src.sha256_url.clone(),
                p.asset_rule_for_os().map(|r| r.urls.clone()).unwrap_or_default(),
            ),
            None => (
                false,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                Vec::new(),
            ),
        };
    ProgramView {
        id: p.id.clone(),
        name: p.name.clone(),
        description: p.description.clone(),
        repo: p.repo.clone(),
        binary: p.binary.clone(),
        args: p.args.clone(),
        fields: p.fields.iter().map(to_field_view).collect(),
        env: p
            .env
            .iter()
            .map(|e| EnvView { key: e.key.clone(), value: e.value.clone(), label: e.label.clone() })
            .collect(),
        hidden: p.hidden,
        http_enabled,
        http_version_url,
        http_version_json_path,
        http_version_regex,
        http_sha256_url,
        http_urls,
    }
}

fn apply_locale(override_locale: Option<&str>) -> String {
    let effective = shared::locale::apply(override_locale, &system_hint()).to_string();
    rust_i18n::set_locale(&effective);
    effective
}

fn system_hint() -> String {
    sys_locale::get_locale().unwrap_or_else(|| "en".to_string())
}

fn program_not_found(id: &str) -> String {
    t!("err.program_not_found", id = id).to_string()
}

/// 从 args 里取字符串参数（missing/null 返回默认）
fn arg_str(args: &serde_json::Map<String, Value>, key: &str) -> String {
    args.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn arg_bool(args: &serde_json::Map<String, Value>, key: &str) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn arg_values(args: &serde_json::Map<String, Value>) -> BTreeMap<String, String> {
    args.get("values")
        .and_then(|v| v.as_object())
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        match v {
                            Value::String(s) => s.clone(),
                            Value::Bool(b) => b.to_string(),
                            Value::Number(n) => n.to_string(),
                            other => other.to_string(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn arg_payload<T: serde::de::DeserializeOwned>(args: &serde_json::Map<String, Value>) -> Result<T, String> {
    let value = args
        .get("payload")
        .ok_or_else(|| "rpc: missing payload arg".to_string())?
        .clone();
    serde_json::from_value(value).map_err(|e| format!("rpc: bad payload: {e}"))
}

// -------- 程序编辑载荷（镜像 app-tauri 的 EditProgramPayload/edit/env 结构） --------

#[derive(Deserialize)]
struct EditField {
    #[serde(default)]
    key: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    default: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    placeholder: String,
}

#[derive(Deserialize)]
struct EditEnv {
    #[serde(default)]
    key: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    label: String,
}

#[derive(Deserialize)]
struct EditProgramPayload {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    repo: String,
    #[serde(default)]
    binary: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    fields: Vec<EditField>,
    #[serde(default)]
    env: Vec<EditEnv>,
    #[serde(default)]
    http_enabled: bool,
    #[serde(default)]
    http_version_url: String,
    #[serde(default)]
    http_version_json_path: String,
    #[serde(default)]
    http_version_regex: String,
    #[serde(default)]
    http_sha256_url: String,
    #[serde(default)]
    http_urls: Vec<String>,
}

/// 由编辑载荷合成新 Program；未编辑项（资产规则/架构映射/工作目录等）沿用 base。
fn build_program_from_edit(e: &EditProgramPayload, base: &Program) -> Program {
    use shared::config::FieldKind;
    let fields = e
        .fields
        .iter()
        .map(|f| {
            let kind = match f.kind.as_str() {
                "file" => FieldKind::File {
                    label: f.label.clone(),
                    default: f.default.clone(),
                    filter: String::new(),
                },
                "directory" => FieldKind::Directory {
                    label: f.label.clone(),
                    default: f.default.clone(),
                },
                "boolean" => FieldKind::Boolean {
                    label: f.label.clone(),
                    default: f.default == "true",
                },
                "autostart" => FieldKind::AutoStart {
                    label: f.label.clone(),
                    default: f.default == "true",
                },
                _ => FieldKind::String {
                    label: f.label.clone(),
                    default: f.default.clone(),
                    placeholder: f.placeholder.clone(),
                },
            };
            Field {
                key: f.key.clone(),
                kind,
                required: f.required,
            }
        })
        .collect();
    let mut assets = base.assets.clone();
    if e.http_enabled {
        let urls: Vec<String> = e
            .http_urls
            .iter()
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .collect();
        if !urls.is_empty() {
            let os = shared::config::os_key();
            match assets.get_mut(os) {
                Some(rule) => rule.urls = urls.clone(),
                None => {
                    let mut def = shared::config::default_os_assets();
                    if let Some(r) = def.get_mut(os) {
                        r.urls = urls.clone();
                    }
                    assets.extend(def);
                }
            }
        }
    }
    let source = if e.http_enabled {
        Some(shared::config::SourceSpec {
            kind: "http".to_string(),
            version_url: e.http_version_url.clone(),
            version_json_path: e.http_version_json_path.clone(),
            version_regex: e.http_version_regex.clone(),
            sha256_url: e.http_sha256_url.clone(),
        })
    } else {
        None
    };
    Program {
        id: e.id.clone(),
        name: e.name.clone(),
        description: e.description.clone(),
        category: base.category.clone(),
        repo: e.repo.clone(),
        source,
        binary: if e.binary.is_empty() {
            e.id.clone()
        } else {
            e.binary.clone()
        },
        assets,
        arch_map: base.arch_map.clone(),
        os_map: base.os_map.clone(),
        fields,
        args: e.args.clone(),
        env: e
            .env
            .iter()
            .filter(|ev| !ev.key.trim().is_empty())
            .map(|ev| shared::config::EnvVar {
                key: ev.key.trim().to_string(),
                value: ev.value.clone(),
                label: ev.label.trim().to_string(),
            })
            .collect(),
        working_dir: base.working_dir.clone(),
        template_source: base.template_source.clone(),
        imported_at: base.imported_at,
        check_sha256: base.check_sha256.clone(),
        hidden: base.hidden,
    }
}

/// 空白 base（新建程序）：仅 id/binary 预填，其余为空。
fn blank_program(id: &str) -> Program {
    Program {
        id: id.to_string(),
        name: String::new(),
        description: String::new(),
        category: String::new(),
        repo: String::new(),
        source: None,
        binary: id.to_string(),
        assets: BTreeMap::new(),
        arch_map: BTreeMap::new(),
        os_map: BTreeMap::new(),
        fields: Vec::new(),
        args: Vec::new(),
        env: Vec::new(),
        working_dir: String::new(),
        template_source: None,
        imported_at: None,
        check_sha256: None,
        hidden: false,
    }
}

/// 本地导入公共落盘：解析好 Program 后按 overwrite 覆盖或追加进受管列表。
fn commit_program(
    mgr: &mut ShellManager,
    program: &mut Program,
    overwrite: bool,
    config_path: &PathBuf,
    import_desc: &str,
) -> Result<ProgramView, String> {
    if program.binary.is_empty() {
        program.binary = program.id.clone();
    }
    if let Some(idx) = mgr.programs.iter().position(|p| p.id == program.id) {
        if !overwrite {
            return Err(t!("err.program_exists", id = &program.id).to_string());
        }
        mgr.programs[idx] = program.clone();
        let view = to_view(program);
        mgr.save_config(config_path).map_err(|e| format!("{e:#}"))?;
        mgr.log_op(&t!("op.import_overwrite", desc = import_desc, id = &program.id));
        return Ok(view);
    }
    let view = to_view(program);
    let name = program.name.clone();
    mgr.programs.push(program.clone());
    mgr.save_config(config_path).map_err(|e| format!("{e:#}"))?;
    mgr.log_op(&t!("op.import", desc = import_desc, name = &name));
    Ok(view)
}

pub async fn dispatch(state: &RpcState, cmd: &str, args: &serde_json::Map<String, Value>) -> RpcResponse {
    let result = handle(state, cmd, args);
    match result {
        Ok(v) => RpcResponse { ok: true, data: Some(v), error: None },
        Err(e) => RpcResponse { ok: false, data: None, error: Some(e) },
    }
}

fn handle(state: &RpcState, cmd: &str, args: &serde_json::Map<String, Value>) -> Result<Value, String> {
    match cmd {
        // ---------- 语言 / 版本 ----------
        "get_locale" => {
            let mgr = state.manager.lock().unwrap();
            let manual = mgr.locale.clone();
            let effective = apply_locale(if manual == "auto" { None } else { Some(&manual) });
            let view = serde_json::to_value(LocaleView {
                effective,
                manual,
                available: shared::locale::LOCALES.iter().map(|s| s.to_string()).collect(),
            })
            .map_err(|e| format!("rpc: locale view: {e}"))?;
            Ok(view)
        }
        "set_locale" => {
            let locale = arg_str(args, "locale");
            let manual = if locale.is_empty() || locale == "auto" {
                "auto".to_string()
            } else if shared::locale::LOCALES.contains(&locale.as_str()) {
                locale
            } else {
                "auto".to_string()
            };
            let mut mgr = state.manager.lock().unwrap();
            mgr.locale = manual.clone();
            let effective = apply_locale(if manual == "auto" { None } else { Some(&manual) });
            mgr.save_config(&state.config_path).map_err(|e| format!("{e:#}"))?;
            Ok(json!({
                "effective": effective,
                "manual": manual,
                "available": shared::locale::LOCALES,
            }))
        }
        "get_shell_version" => Ok(json!(shared::version::build_version())),

        // ---------- 程序视图 / 值 ----------
        "get_programs" => {
            let mgr = state.manager.lock().unwrap();
            Ok(serde_json::to_value(
                mgr.all_programs().iter().map(to_view).collect::<Vec<ProgramView>>(),
            )
            .unwrap_or(json!([])))
        }
        "get_values" => {
            let program_id = arg_str(args, "programId");
            let mgr = state.manager.lock().unwrap();
            let Some(p) = mgr.all_programs().into_iter().find(|p| p.id == program_id) else {
                return Err(program_not_found(&program_id));
            };
            Ok(serde_json::to_value(mgr.load_field_values(&p)).unwrap_or(json!({})))
        }
        "save_values" => {
            let program_id = arg_str(args, "programId");
            let mgr = state.manager.lock().unwrap();
            let Some(p) = mgr.all_programs().into_iter().find(|p| p.id == program_id) else {
                return Err(program_not_found(&program_id));
            };
            mgr.save_field_values(&p, &arg_values(args));
            Ok(json!({}))
        }

        // ---------- 状态 ----------
        "get_status_local" => {
            let program_id = arg_str(args, "programId");
            let mut mgr = state.manager.lock().unwrap();
            let Some(p) = mgr.all_programs().into_iter().find(|p| p.id == program_id) else {
                return Err(program_not_found(&program_id));
            };
            let bin = mgr.bin_path(&p);
            let s = mgr.status_local(&p);
            let auto = mgr.program_autostart(&p.id);
            let vcheck = mgr.load_version_check();
            Ok(serde_json::to_value(StatusView::from_local(&s, &bin, auto, &p.repo, &vcheck)).unwrap_or(json!({})))
        }
        "batch_status_local" => {
            let mut mgr = state.manager.lock().unwrap();
            let progs = mgr.all_programs();
            let vcheck = mgr.load_version_check();
            let views: Vec<ProgramStatusView> = progs
                .into_iter()
                .map(|p| {
                    let bin = mgr.bin_path(&p);
                    let s = mgr.status_local(&p);
                    let auto = mgr.program_autostart(&p.id);
                    ProgramStatusView {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        repo: p.repo.clone(),
                        hidden: p.hidden,
                        status: StatusView::from_local(&s, &bin, auto, &p.repo, &vcheck),
                    }
                })
                .collect();
            Ok(serde_json::to_value(views).unwrap_or(json!([])))
        }

        // ---------- 启停 ----------
        "start_program" => {
            let program_id = arg_str(args, "programId");
            let mut mgr = state.manager.lock().unwrap();
            let Some(p) = mgr.all_programs().into_iter().find(|p| p.id == program_id) else {
                return Err(program_not_found(&program_id));
            };
            let values = arg_values(args);
            mgr.save_field_values(&p, &values);
            if let Err(e) = mgr.apply_key_autostart(&p, &values) {
                mgr.log_op(&t!("log.autostart.set_fail", err = format!("{e:#}")).to_string());
            }
            mgr.start(&p, &values).map_err(|e| format!("{e:#}"))?;
            mgr.log_op(&t!("op.start", name = &p.name).to_string());
            let bin = mgr.bin_path(&p);
            let s = mgr.status_local(&p);
            let auto = mgr.program_autostart(&p.id);
            let vcheck = mgr.load_version_check();
            Ok(serde_json::to_value(StatusView::from_local(&s, &bin, auto, &p.repo, &vcheck)).unwrap_or(json!({})))
        }
        "stop_program" => {
            let program_id = arg_str(args, "programId");
            let mut mgr = state.manager.lock().unwrap();
            let Some(p) = mgr.all_programs().into_iter().find(|p| p.id == program_id) else {
                return Err(program_not_found(&program_id));
            };
            mgr.stop(&program_id).map_err(|e| format!("{e:#}"))?;
            mgr.clear_log(&program_id);
            mgr.log_op(&t!("op.stop", name = &p.name).to_string());
            let bin = mgr.bin_path(&p);
            let s = mgr.status_local(&p);
            let auto = mgr.program_autostart(&p.id);
            let vcheck = mgr.load_version_check();
            Ok(serde_json::to_value(StatusView::from_local(&s, &bin, auto, &p.repo, &vcheck)).unwrap_or(json!({})))
        }
        "restart_program" => {
            let program_id = arg_str(args, "programId");
            let mut mgr = state.manager.lock().unwrap();
            let Some(p) = mgr.all_programs().into_iter().find(|p| p.id == program_id) else {
                return Err(program_not_found(&program_id));
            };
            let values = arg_values(args);
            mgr.stop(&p.id).map_err(|e| format!("{e:#}"))?;
            mgr.save_field_values(&p, &values);
            if let Err(e) = mgr.apply_key_autostart(&p, &values) {
                mgr.log_op(&t!("log.autostart.set_fail", err = format!("{e:#}")).to_string());
            }
            mgr.start(&p, &values).map_err(|e| format!("{e:#}"))?;
            mgr.log_op(&t!("op.restart", name = &p.name).to_string());
            let bin = mgr.bin_path(&p);
            let s = mgr.status_local(&p);
            let auto = mgr.program_autostart(&p.id);
            let vcheck = mgr.load_version_check();
            Ok(serde_json::to_value(StatusView::from_local(&s, &bin, auto, &p.repo, &vcheck)).unwrap_or(json!({})))
        }
        "stop_all" => {
            let mut mgr = state.manager.lock().unwrap();
            mgr.stop_all();
            mgr.log_op(&t!("op.stop_all").to_string());
            Ok(json!({}))
        }

        // ---------- 日志 ----------
        "get_logs" => {
            let program_id = arg_str(args, "programId");
            let mgr = state.manager.lock().unwrap();
            if !mgr.all_programs().iter().any(|p| p.id == program_id) {
                return Err(program_not_found(&program_id));
            }
            let (out, _err) = mgr.read_logs(&program_id, 64 * 1024);
            Ok(serde_json::to_value(LogsView { text: out }).map_err(|e| format!("rpc: logs view: {e}"))?)
        }
        "get_shell_log" => {
            let mgr = state.manager.lock().unwrap();
            let path = mgr.op_log_path();
            if !path.exists() {
                return Ok(json!(""));
            }
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{e:#}"))?;
            const KEEP: usize = 400;
            let lines: Vec<&str> = text.lines().collect();
            let start = lines.len().saturating_sub(KEEP);
            Ok(json!(lines[start..].join("\n")))
        }
        "log_shell_op" => {
            let msg = arg_str(args, "msg");
            let mgr = state.manager.lock().unwrap();
            mgr.log_op(&msg);
            Ok(json!({}))
        }
        "clear_shell_log" => {
            let mgr = state.manager.lock().unwrap();
            mgr.clear_op_log();
            Ok(json!({}))
        }

        // ---------- 代理 ----------
        "get_proxy" => {
            let mgr = state.manager.lock().unwrap();
            Ok(json!(ProxyView {
                accelerate_prefix: mgr.proxy.accelerate_prefix.clone(),
                http_proxy: mgr.proxy.http_proxy.clone(),
            }))
        }
        "set_proxy" => {
            let mut mgr = state.manager.lock().unwrap();
            mgr.proxy.accelerate_prefix = arg_str(args, "acceleratePrefix").trim().to_string();
            mgr.proxy.http_proxy = arg_str(args, "httpProxy").trim().to_string();
            let (acc, hp) = (mgr.proxy.accelerate_prefix.clone(), mgr.proxy.http_proxy.clone());
            mgr.github.apply_network(&acc, &hp);
            shared::clear_github_cache();
            let reg_cache = mgr.data_dir.join("cache/registry");
            let _ = std::fs::remove_dir_all(&reg_cache);
            mgr.save_config(&state.config_path).map_err(|e| format!("{e:#}"))?;
            Ok(json!({}))
        }

        // ---------- 自启 ----------
        "set_autostart" => {
            let program_id = arg_str(args, "programId");
            let enabled = arg_bool(args, "enabled");
            let mut mgr = state.manager.lock().unwrap();
            let Some(p) = mgr.all_programs().into_iter().find(|p| p.id == program_id) else {
                return Err(program_not_found(&program_id));
            };
            mgr.set_program_autostart(&program_id, enabled);
            mgr.log_op(&t!(
                "op.toggle_autostart",
                onoff = t!(if enabled { "op.enable" } else { "op.disable" }),
                name = &p.name
            )
            .to_string());
            Ok(json!({}))
        }
        "shell_autostart_enabled" => {
            let mgr = state.manager.lock().unwrap();
            Ok(json!(mgr.autostart.shell_is_enabled()))
        }
        "set_shell_autostart" => {
            let enabled = arg_bool(args, "enabled");
            let mut mgr = state.manager.lock().unwrap();
            let r = mgr.autostart.set_shell_enabled(enabled).map_err(|e| format!("{e:#}"));
            if r.is_ok() {
                mgr.log_op(&t!(
                    "op.toggle_shell_autostart",
                    onoff = t!(if enabled { "op.enable" } else { "op.disable" })
                )
                .to_string());
            }
            r.map(|_| json!({}))
        }

        // ---------- reveal（服务端原生能力，本机语义） ----------
        "reveal_logs" | "reveal_logs_dir" => {
            let mgr = state.manager.lock().unwrap();
            let log_dir = mgr.data_dir.join("logs");
            std::fs::create_dir_all(&log_dir).map_err(|e| format!("{e:#}"))?;
            open_in_file_manager(log_dir)
                .map_err(|e| e.to_string())?;
            Ok(json!({}))
        }
        "reveal_app_dir" => {
            let program_id = arg_str(args, "programId");
            let mgr = state.manager.lock().unwrap();
            let dir = mgr.data_dir.join(&program_id);
            std::fs::create_dir_all(&dir).map_err(|e| format!("{e:#}"))?;
            open_in_file_manager(dir)
                .map_err(|e| e.to_string())?;
            Ok(json!({}))
        }

        // ---------- 程序定义编辑（新增/修改/复制/删除/显隐） ----------
        "edit_program" => {
            let payload: EditProgramPayload = arg_payload(args)?;
            let mut mgr = state.manager.lock().unwrap();
            let base = mgr
                .all_programs()
                .into_iter()
                .find(|p| p.id == payload.id)
                .ok_or_else(|| program_not_found(&payload.id))?;
            let updated = build_program_from_edit(&payload, &base);
            mgr.update_program(&payload.id, &updated, &state.config_path)
                .map_err(|e| format!("{e:#}"))?;
            mgr.log_op(&t!("op.edit_template", name = &base.name));
            Ok(serde_json::to_value(to_view(&updated)).map_err(|e| format!("rpc: view: {e}"))?)
        }
        "add_program" => {
            let payload: EditProgramPayload = arg_payload(args)?;
            if payload.id.trim().is_empty() {
                return Err(t!("err.empty_id").to_string());
            }
            let blank = blank_program(&payload.id);
            let created = build_program_from_edit(&payload, &blank);
            let mut mgr = state.manager.lock().unwrap();
            mgr.add_program(&created, &state.config_path)
                .map_err(|e| format!("{e:#}"))?;
            mgr.save_field_values(&created, &BTreeMap::new());
            mgr.log_op(&t!("op.add_template", name = &created.name));
            Ok(serde_json::to_value(to_view(&created)).map_err(|e| format!("rpc: view: {e}"))?)
        }
        "duplicate_program" => {
            let program_id = arg_str(args, "programId");
            let mut mgr = state.manager.lock().unwrap();
            let copy = mgr
                .duplicate_program(&program_id, &state.config_path)
                .map_err(|e| format!("{e:#}"))?;
            mgr.log_op(&t!("op.duplicate", name = &copy.name));
            Ok(serde_json::to_value(to_view(&copy)).map_err(|e| format!("rpc: view: {e}"))?)
        }
        "delete_program" => {
            let program_id = arg_str(args, "programId");
            let mut mgr = state.manager.lock().unwrap();
            let name = mgr
                .programs
                .iter()
                .find(|p| p.id == program_id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| program_id.clone());
            mgr.delete_program(&program_id, &state.config_path)
                .map_err(|e| format!("{e:#}"))?;
            mgr.log_op(&t!("op.delete", name = &name));
            Ok(json!({}))
        }
        "set_program_hidden" => {
            let program_id = arg_str(args, "programId");
            let hidden = arg_bool(args, "hidden");
            let mut mgr = state.manager.lock().unwrap();
            let name = mgr
                .programs
                .iter()
                .find(|p| p.id == program_id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| program_id.clone());
            mgr.set_hidden(&program_id, hidden, &state.config_path)
                .map_err(|e| format!("{e:#}"))?;
            mgr.log_op(&t!(
                "op.toggle_visibility",
                showhide = t!(if hidden { "op.hide" } else { "op.show" }),
                name = &name
            ));
            Ok(json!({}))
        }
        "import_template_json" => {
            let text = arg_str(args, "templateJson");
            let overwrite = arg_bool(args, "overwrite");
            let mut program: Program = serde_json::from_str(&text)
                .map_err(|e| t!("err.parse_template_fail", err = e.to_string()).to_string())?;
            let mut mgr = state.manager.lock().unwrap();
            let view = commit_program(
                &mut mgr,
                &mut program,
                overwrite,
                &state.config_path,
                &t!("op.import_web"),
            )?;
            Ok(serde_json::to_value(view).map_err(|e| format!("rpc: view: {e}"))?)
        }

        other => Err(format!("rpc: unknown command {other}")),
    }
}

#[cfg(target_os = "macos")]
fn open_in_file_manager(path: PathBuf) -> anyhow::Result<()> {
    std::process::Command::new("open").arg(&path).spawn().map(|_| ()).map_err(Into::into)
}

#[cfg(target_os = "linux")]
fn open_in_file_manager(path: PathBuf) -> anyhow::Result<()> {
    std::process::Command::new("xdg-open").arg(&path).spawn().map(|_| ()).map_err(Into::into)
}

#[cfg(target_os = "windows")]
fn open_in_file_manager(path: PathBuf) -> anyhow::Result<()> {
    std::process::Command::new("explorer").arg(&path).spawn().map(|_| ()).map_err(Into::into)
}