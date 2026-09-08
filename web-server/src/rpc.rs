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