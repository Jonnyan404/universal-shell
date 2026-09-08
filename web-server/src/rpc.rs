//! 面向浏览器的 JSON-RPC 层：`POST /api/rpc`，命令与 app-tauri 后端一一对应
//! （镜像同名命令的参数/返回结构），供同一份 SPA 在浏览器与桌面窗口使用。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use shared::config::{Field, Program};
use shared::ShellManager;

use rust_i18n::t;

/// 鉴权失败文案（F6）。供 lib.rs 中间件复用。
pub fn token_required(_token: &str) -> String {
    t!("err.web_token_required").to_string()
}

/// 原生文件选择器（F9）：egui/tauri 宿主进程启动时调用 `enable_native_pick()`；
/// 纯浏览器上下文（独立 web 服务）不启用 → SPA 回退为手填路径。
pub static NATIVE_PICK: AtomicBool = AtomicBool::new(false);

pub fn enable_native_pick() {
    NATIVE_PICK.store(true, Ordering::Relaxed);
}

/// 共享状态：宿主进程(egui / tauri)把同一份 manager 交进来。
/// `events` 是 WebSocket 事件总线（下载进度等后台任务向浏览器广播，F-3 事件推送复用）。
pub struct RpcState {
    pub manager: Arc<Mutex<ShellManager>>,
    pub config_path: PathBuf,
    pub events: tokio::sync::broadcast::Sender<String>,
    /// 访问令牌：非回环 peer 必须携带（F6 鉴权）。首次启动生成并持久化。
    pub token: String,
}

impl RpcState {
    pub fn new(manager: Arc<Mutex<ShellManager>>, config_path: PathBuf) -> Self {
        let (tx, _rx) = tokio::sync::broadcast::channel(128);
        let token = {
            let mut mgr = manager.lock().unwrap();
            if mgr.web.token.is_empty() {
                mgr.web.token = generate_token();
                let _ = mgr.save_config(&config_path);
            }
            mgr.web.token.clone()
        };
        Self { manager, config_path, events: tx, token }
    }
}

/// 生成 48 位十六进制随机令牌（URL 安全字符集）。
pub fn generate_token() -> String {
    let mut bytes = [0u8; 24];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
    /// 本次返回时文件 EOF 字节偏移（前端据此做增量尾随）
    offset: u64,
    /// 文件被截断/重建导致偏移失效，前端应整体重渲
    reset: bool,
}

/// 增量日志读取（F-3/F11）：
/// - 无 offset：返回最近 64KB 尾部 + EOF 偏移（前端首次加载）
/// - 有 offset 且 <= len：只返回自 offset 起新增字节（delta，不整页重传）
/// - 有 offset 但文件已截断/重建：返回尾部 + 新偏移 + reset=true
fn read_log_delta(path: &std::path::Path, offset: Option<u64>) -> (String, u64, bool) {
    const TAIL: u64 = 64 * 1024;
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return (String::new(), offset.unwrap_or(0), false);
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return (String::new(), offset.unwrap_or(0), false);
    };
    fn read_tail(f: &mut std::fs::File, len: u64) -> (Vec<u8>, u64) {
        use std::io::{Read, Seek, SeekFrom};
        let start = len.saturating_sub(TAIL);
        if f.seek(SeekFrom::Start(start)).is_err() {
            return (Vec::new(), len);
        }
        let mut v = Vec::new();
        let _ = f.read_to_end(&mut v);
        if start > 0 {
            if let Some(i) = v.iter().position(|&b| b == b'\n') {
                v = v[(i + 1)..].to_vec();
            }
        }
        (v, len)
    }
    match offset {
        None => {
            let (v, len) = read_tail(&mut f, len);
            (String::from_utf8_lossy(&v).to_string(), len, false)
        }
        Some(off) if off == len => (String::new(), len, false),
        Some(off) if off < len => {
            let _ = f.seek(SeekFrom::Start(off));
            let mut v = Vec::new();
            let _ = f.read_to_end(&mut v);
            (String::from_utf8_lossy(&v).to_string(), len, false)
        }
        Some(_) => {
            let (v, len) = read_tail(&mut f, len);
            (String::from_utf8_lossy(&v).to_string(), len, true)
        }
    }
}

impl StatusView {
    fn now_unix() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// 远端已刷新的完整状态（批量管理用）：直接带最新版本与发布时间
    fn from_status(s: &shared::ProgramStatus, bin_path: &PathBuf, autostart: bool) -> Self {
        let up_to_date = s.installed
            && s.local_version != "-"
            && match &s.latest_version {
                Some(lv) => !shared::version::is_newer(lv, &s.local_version),
                None => true,
            };
        Self {
            installed: s.installed,
            running: s.running,
            autostart,
            local_version: s.local_version.clone(),
            latest_version: s.latest_version.clone(),
            latest_published: s.latest_published.clone(),
            latest_checked_at: s.latest_version.as_ref().map(|_| Self::now_unix()),
            up_to_date,
            bin_path: bin_path.display().to_string(),
        }
    }

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

/// 用当前网络设置(加速前缀 + 通用代理)构建 GitHub 客户端
fn proxied_github(proxy: &shared::ProxySettings) -> shared::GitHub {
    let mut gh = shared::GitHub::default();
    gh.apply_network(&proxy.accelerate_prefix, &proxy.http_proxy);
    gh
}

fn proxied_http(proxy: &shared::ProxySettings) -> shared::source_http::HttpSource {
    let mut hs = shared::source_http::HttpSource::default();
    hs.apply_network(&proxy.accelerate_prefix, &proxy.http_proxy);
    hs
}

/// 构造绑定某源的 RegistryClient（模板校验/pubkey/代理）
fn registry_client(
    mgr: &ShellManager,
    registry_url: &str,
) -> shared::RegistryClient {
    shared::RegistryClient::with_network(
        registry_url,
        mgr.data_dir.join("cache/registry"),
        mgr.registry_pubkeys.clone(),
        Some(&mgr.proxy.accelerate_prefix),
        Some(&mgr.proxy.http_proxy),
    )
}

#[derive(Serialize)]
struct MergedManifestView {
    templates: Vec<(String, shared::TemplateIndex, String)>, // (id, index, base)
    sources: Vec<(String, bool, u64)>,                      // (base, offline, fetched_at)
    conflicts: Vec<(String, usize)>,                        // id -> 源数量
}

fn merged_manifest_view(merged: &shared::MergedSource) -> MergedManifestView {
    MergedManifestView {
        templates: merged
            .by_id
            .iter()
            .map(|(id, (base, idx))| (id.clone(), idx.clone(), base.clone()))
            .collect(),
        sources: merged.sources.clone(),
        conflicts: merged
            .conflicts
            .iter()
            .map(|(id, bases)| (id.clone(), bases.len()))
            .collect(),
    }
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
    let state = RpcState {
        manager: state.manager.clone(),
        config_path: state.config_path.clone(),
        events: state.events.clone(),
        token: state.token.clone(),
    };
    let cmd = cmd.to_string();
    let args = args.clone();
    // 同步逻辑（可能含网络/锁竞争）放 blocking 池，避免占住 tokio worker 导致其它请求饿死
    match tokio::task::spawn_blocking(move || handle(&state, &cmd, &args)).await {
        Ok(result) => match result {
            Ok(v) => RpcResponse { ok: true, data: Some(v), error: None },
            Err(e) => RpcResponse { ok: false, data: None, error: Some(e) },
        },
        Err(_) => RpcResponse {
            ok: false,
            data: None,
            error: Some("rpc: handler panicked".to_string()),
        },
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
        "get_theme" => {
            let mgr = state.manager.lock().unwrap();
            Ok(json!({ "theme": mgr.theme }))
        }
        "set_theme" => {
            let theme = arg_str(args, "theme");
            let manual = match theme.as_str() {
                "light" | "dark" => theme,
                _ => "auto".to_string(),
            };
            let mut mgr = state.manager.lock().unwrap();
            mgr.theme = manual.clone();
            mgr.save_config(&state.config_path).map_err(|e| format!("{e:#}"))?;
            Ok(json!({ "theme": manual }))
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
            let offset = args.get("offset").and_then(|v| v.as_u64());
            let mgr = state.manager.lock().unwrap();
            if !mgr.all_programs().iter().any(|p| p.id == program_id) {
                return Err(program_not_found(&program_id));
            }
            let path = mgr.log_dir().join(format!("{program_id}.log"));
            let (text, new_offset, reset) = read_log_delta(&path, offset);
            Ok(serde_json::to_value(LogsView { text, offset: new_offset, reset }).map_err(|e| format!("rpc: logs view: {e}"))?)
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
        "open_url" => {
            let url = arg_str(args, "url");
            open_external(&url).map_err(|e| e.to_string())?;
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

        // ---------- Web 监听设置（端口/绑定） ----------
        "get_web_settings" => {
            let mgr = state.manager.lock().unwrap();
            let w = mgr.web_settings();
            Ok(json!({ "bind": w.bind, "port": w.port, "token": w.token }))
        }
        "set_web_settings" => {
            let bind = arg_str(args, "bind");
            let port = args.get("port").and_then(|v| v.as_u64()).unwrap_or(0).min(65535) as u16;
            // 自定义密码：args.token 提供则覆盖；空串表示自动生成新随机令牌；
            // 缺省则保持原值
            let token = match args.get("token") {
                Some(v) => v.as_str().map(|s| if s.trim().is_empty() { generate_token() } else { s.trim().to_string() }),
                None => None,
            };
            let mut mgr = state.manager.lock().unwrap();
            mgr.set_web_settings(&bind, port, token.as_deref());
            mgr.save_config(&state.config_path).map_err(|e| format!("{e:#}"))?;
            let bind_show = if bind.is_empty() { "127.0.0.1".to_string() } else { bind };
            mgr.log_op(&t!("op.web_update", bind = bind_show, port = port.to_string()));
            Ok(json!({}))
        }

        // ---------- 原生路径选择（F9）：本机窗口走服务端 rfd；远程/无 GUI 返回 supported:false ----------
        "pick_file" => {
            if !NATIVE_PICK.load(Ordering::Relaxed) {
                return Ok(json!({ "supported": false }));
            }
            let mut dialog = rfd::FileDialog::new().set_title(t!("dlg.pick_binary").into_owned());
            if let Ok(cwd) = std::env::current_dir() {
                dialog = dialog.set_directory(cwd);
            }
            let picked = dialog.pick_file();
            let path = picked.map(|p| p.to_string_lossy().to_string());
            Ok(json!({ "supported": true, "path": path }))
        }

        // 原生目录选择：供 file/directory 类型字段选择工作目录/数据目录
        "pick_directory" => {
            if !NATIVE_PICK.load(Ordering::Relaxed) {
                return Ok(json!({ "supported": false }));
            }
            let mut dialog = rfd::FileDialog::new().set_title(t!("dlg.pick_directory").into_owned());
            if let Ok(cwd) = std::env::current_dir() {
                dialog = dialog.set_directory(cwd);
            }
            let picked = dialog.pick_folder();
            let path = picked.map(|p| p.to_string_lossy().to_string());
            Ok(json!({ "supported": true, "path": path }))
        }

        // ---------- 批量管理：远端最新版本 ----------
        "batch_status" => {
            let (layout, locals) = {
                let mut mgr = state.manager.lock().unwrap();
                (
                    mgr.proxy.clone(),
                    mgr.all_programs()
                        .into_iter()
                        .map(|p| {
                            let bin = mgr.bin_path(&p);
                            let s = mgr.status_local(&p);
                            let auto = mgr.program_autostart(&p.id);
                            (p, bin, s, auto)
                        })
                        .collect::<Vec<_>>(),
                )
            };
            // 锁外并行：按 source 分发查最新版本
            let latest: Vec<Option<(String, String)>> = std::thread::scope(|s| {
                let handles: Vec<_> = locals
                    .iter()
                    .map(|(p, _, _, _)| {
                        let p = p.clone();
                        let layout = layout.clone();
                        s.spawn(move || {
                            let gh = proxied_github(&layout);
                            let hs = proxied_http(&layout);
                            shared::shell_manager::latest_remote(&p, &gh, &hs)
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap_or(None)).collect()
            });
            let mut locals = locals;
            for ((_, _, s, _), lv) in locals.iter_mut().zip(latest) {
                if let Some((v, pb)) = lv {
                    s.latest_version = Some(v);
                    s.latest_published = pb;
                }
            }
            if locals.iter().any(|(_, _, s, _)| s.latest_version.is_some()) {
                let mgr = state.manager.lock().unwrap();
                let mut vc = mgr.load_version_check();
                for (p, _, s, _) in locals.iter() {
                    if let Some(v) = &s.latest_version {
                        vc.insert(p.repo.clone(), (v.clone(), StatusView::now_unix()));
                    }
                }
                mgr.save_version_check(&vc);
            }
            let views: Vec<ProgramStatusView> = locals
                .into_iter()
                .map(|(p, bin, s, auto)| ProgramStatusView {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    repo: p.repo.clone(),
                    hidden: p.hidden,
                    status: StatusView::from_status(&s, &bin, auto),
                })
                .collect();
            Ok(serde_json::to_value(views).map_err(|e| format!("rpc: view: {e}"))?)
        }

        // ---------- 下载安装（后台进度经 WS 事件总线广播） ----------
        "install" => {
            let program_id = arg_str(args, "programId");
            let (data_dir, program, events) = {
                let mgr = state.manager.lock().unwrap();
                let p = mgr
                    .all_programs()
                    .into_iter()
                    .find(|p| p.id == program_id)
                    .ok_or_else(|| program_not_found(&program_id))?;
                (mgr.data_dir.clone(), p, state.events.clone())
            };
            std::thread::spawn(move || {
                use shared::progress::{DownloadProgress, DownloadStage};
                let pid = program.id.clone();
                let stage_name = |st: &DownloadStage| match st {
                    DownloadStage::Downloading => "downloading",
                    DownloadStage::Verifying => "verifying",
                    DownloadStage::Extracting => "extracting",
                };
                let send = |stage: &str,
                            received: u64,
                            total: u64,
                                done: bool,
                                error: Option<&str>,
                                version: Option<&str>| {
                    let _ = events.send(json!({
                        "type": "install-progress",
                        "programId": pid,
                        "stage": stage,
                        "received": received,
                        "total": total,
                        "done": done,
                        "error": error,
                        "version": version,
                    }).to_string());
                };
                let on_progress = |pr: &DownloadProgress| {
                    send(
                        stage_name(&pr.stage),
                        pr.received,
                        pr.total,
                        false,
                        None,
                        None,
                    );
                };
                match shared::ShellManager::install_standalone_with_progress(&data_dir, &program, &on_progress) {
                    Ok(version) => send("done", 0, 0, true, None, Some(&version)),
                    Err(e) => {
                        let err_text = format!("{e:#}");
                        shared::ShellManager::log_op_for(
                            &data_dir,
                            &t!("op.download_fail", name = &program.name, err = &err_text),
                        );
                        send("error", 0, 0, false, Some(&err_text), None);
                    }
                }
            });
            Ok(json!({}))
        }

        // ---------- 壳更新检查 ----------
        "check_shell_update" => {
            let (accel, proxy) = {
                let mgr = state.manager.lock().unwrap();
                (mgr.proxy.accelerate_prefix.clone(), mgr.proxy.http_proxy.clone())
            };
            let current = shared::version::build_version().to_string();
            match shared::check_shell_update(&current, &accel, &proxy).map_err(|e| format!("{e:#}")) {
                Ok(Some(u)) => Ok(json!({
                    "current": u.current,
                    "latest_tag": Some(u.latest_tag),
                    "release_url": Some(u.release_url),
                })),
                Ok(None) => Ok(json!({
                    "current": current,
                    "latest_tag": None::<String>,
                    "release_url": None::<String>,
                })),
                Err(e) => Err(e),
            }
        }

        // ---------- 模板库 ----------
        "get_registries" => {
            let mgr = state.manager.lock().unwrap();
            Ok(json!(mgr.template_registries.clone()))
        }
        "set_registries" => {
            let registries = args
                .get("registries")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| {
                            let s = s.trim().to_string();
                            if s.ends_with('/') { s } else { format!("{s}/") }
                        })
                        .filter(|s| !s.is_empty() && s != "/")
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut mgr = state.manager.lock().unwrap();
            mgr.template_registries = registries.clone();
            mgr.save_config(&state.config_path).map_err(|e| format!("{e:#}"))?;
            mgr.log_op(&t!("op.update_sources", list = registries.join(", ")));
            Ok(json!({}))
        }
        "get_manifest" => {
            let registry_url = arg_str(args, "registryUrl");
            let (mgr, client) = {
                let mgr = state.manager.lock().unwrap();
                let client = registry_client(&mgr, &registry_url);
                (mgr, client)
            };
            drop(mgr);
            let (offline, _fetched_at, manifest) = client
                .load_manifest()
                .map_err(|e| format!("{e:#}"))?;
            Ok(json!({
                "revision": manifest.revision,
                "categories": manifest.categories,
                "templates": manifest.templates,
                "offline": offline,
            }))
        }
        "get_merged_manifest" => {
            let (cache, bases0, pubkeys, proxy) = {
                let mgr = state.manager.lock().unwrap();
                (
                    mgr.data_dir.join("cache/registry"),
                    mgr.template_registries.clone(),
                    mgr.registry_pubkeys.clone(),
                    mgr.proxy.clone(),
                )
            };
            let mut bases = bases0;
            let typed = arg_str(args, "registryUrl");
            if !typed.is_empty() && !bases.contains(&typed) {
                bases.push(typed);
            }
            if bases.is_empty() {
                return Err(t!("err.registry_not_configured").to_string());
            }
            let merged = shared::load_merged_manifests(
                &bases,
                cache,
                pubkeys,
                Some(&proxy.accelerate_prefix),
                Some(&proxy.http_proxy),
                true,
            );
            Ok(serde_json::to_value(merged_manifest_view(&merged)).map_err(|e| format!("rpc: view: {e}"))?)
        }
        "get_merged_manifest_offline" => {
            let (cache, bases, pubkeys) = {
                let mgr = state.manager.lock().unwrap();
                (
                    mgr.data_dir.join("cache/registry"),
                    mgr.template_registries.clone(),
                    mgr.registry_pubkeys.clone(),
                )
            };
            let merged = shared::load_merged_manifests_cached(&bases, cache, pubkeys);
            if merged.by_id.is_empty() {
                return Err(t!("err.no_cache_manifest").to_string());
            }
            Ok(serde_json::to_value(merged_manifest_view(&merged)).map_err(|e| format!("rpc: view: {e}"))?)
        }
        "template_status" => {
            let registry_url = arg_str(args, "registryUrl");
            let template_id = arg_str(args, "templateId");
            let (mgr, client) = {
                let mgr = state.manager.lock().unwrap();
                let client = registry_client(&mgr, &registry_url);
                (mgr, client)
            };
            let (_offline, program) = client
                .load_template(&template_id)
                .map_err(|e| format!("{e:#}"))?;
            let status = match mgr.all_programs().into_iter().find(|p| p.id == program.id) {
                None => "new".to_string(),
                Some(cur) => {
                    let diff = shared::ShellManager::template_diff(&cur, &program);
                    if diff.is_empty() { "current".to_string() } else { "update".to_string() }
                }
            };
            Ok(json!(status))
        }
        "template_diff" => {
            let registry_url = arg_str(args, "registryUrl");
            let template_id = arg_str(args, "templateId");
            let (mgr, client) = {
                let mgr = state.manager.lock().unwrap();
                let client = registry_client(&mgr, &registry_url);
                (mgr, client)
            };
            let (_offline, program) = client
                .load_template(&template_id)
                .map_err(|e| format!("{e:#}"))?;
            match mgr.all_programs().into_iter().find(|p| p.id == program.id) {
                None => Err(t!("err.program_exists", id = template_id).to_string()),
                Some(cur) => Ok(serde_json::to_value(shared::ShellManager::template_diff_view(&cur, &program))
                    .map_err(|e| format!("rpc: view: {e}"))?),
            }
        }
        "import_template" => {
            let registry_url = arg_str(args, "registryUrl");
            let template_id = arg_str(args, "templateId");
            let overwrite = arg_bool(args, "overwrite");
            let mut mgr = state.manager.lock().unwrap();
            let client = registry_client(&mgr, &registry_url);
            let (_offline, mut program) = client
                .load_template(&template_id)
                .map_err(|e| format!("{e:#}"))?;
            if let Some(idx) = mgr.programs.iter().position(|p| p.id == program.id) {
                if !overwrite {
                    return Err(t!("err.program_exists", id = &program.id).to_string());
                }
                let cur = mgr.programs[idx].clone();
                let mut next = cur.clone();
                let values = mgr.load_field_values(&cur);
                let merged = shared::ShellManager::apply_template_update(&mut next, &program, &values);
                mgr.save_field_values(&next, &merged);
                mgr.programs[idx] = next.clone();
                let view = to_view(&next);
                mgr.save_config(&state.config_path).map_err(|e| format!("{e:#}"))?;
                mgr.log_op(&t!("op.import_overwrite", desc = t!("op.import_local"), id = &program.id));
                return Ok(serde_json::to_value(view).map_err(|e| format!("rpc: view: {e}"))?);
            }
            if program.binary.is_empty() {
                program.binary = program.id.clone();
            }
            let view = to_view(&program);
            let id = program.id.clone();
            mgr.programs.push(program);
            mgr.save_config(&state.config_path).map_err(|e| format!("{e:#}"))?;
            mgr.log_op(&t!("op.import", desc = t!("op.import_local"), name = &id));
            Ok(serde_json::to_value(view).map_err(|e| format!("rpc: view: {e}"))?)
        }
        "export_template_json" => {
            let program_id = arg_str(args, "programId");
            let mgr = state.manager.lock().unwrap();
            let p = mgr
                .all_programs()
                .into_iter()
                .find(|p| p.id == program_id)
                .ok_or_else(|| program_not_found(&program_id))?;
            let json = serde_json::to_string_pretty(&p).map_err(|e| format!("{e:#}"))?;
            mgr.log_op(&t!("op.export", name = &p.name));
            Ok(json!(json))
        }

        other => Err(format!("rpc: unknown command {other}")),
    }
}

#[cfg(target_os = "macos")]
fn open_in_file_manager(path: PathBuf) -> anyhow::Result<()> {
    std::process::Command::new("open").arg(&path).spawn().map(|_| ()).map_err(Into::into)
}

#[cfg(target_os = "macos")]
fn open_external(url: &str) -> anyhow::Result<()> {
    std::process::Command::new("open").arg(url).spawn().map(|_| ()).map_err(Into::into)
}

#[cfg(target_os = "linux")]
fn open_in_file_manager(path: PathBuf) -> anyhow::Result<()> {
    std::process::Command::new("xdg-open").arg(&path).spawn().map(|_| ()).map_err(Into::into)
}

#[cfg(target_os = "linux")]
fn open_external(url: &str) -> anyhow::Result<()> {
    std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ()).map_err(Into::into)
}

#[cfg(target_os = "windows")]
fn open_in_file_manager(path: PathBuf) -> anyhow::Result<()> {
    std::process::Command::new("explorer").arg(&path).spawn().map(|_| ()).map_err(Into::into)
}

#[cfg(target_os = "windows")]
fn open_external(url: &str) -> anyhow::Result<()> {
    std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn().map(|_| ()).map_err(Into::into)
}