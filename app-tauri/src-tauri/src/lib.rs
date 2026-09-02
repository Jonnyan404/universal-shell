//! universal-shell (Tauri 版后端)
//!
//! 复用 shared core。前端通过 invoke 调这些命令；字段表单由
//! get_programs 返回的 fields 描述动态渲染。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use shared::config::{Field, Program};
use shared::ShellManager;
use tauri::Manager;
use tauri::State;

struct AppState {
    manager: Mutex<ShellManager>,
    config_path: PathBuf,
}

/// 给前端传的字段（含默认值，但不含运行时值——值由 get_values 单独返回）
#[derive(serde::Serialize)]
struct FieldView {
    key: String,
    label: String,
    kind: String,
    default: String,
}

/// 程序描述（前端据此渲染整张表单）
#[derive(serde::Serialize)]
struct ProgramView {
    id: String,
    name: String,
    description: String,
    repo: String,
    binary: String,
    args: Vec<String>,
    fields: Vec<FieldView>,
    hidden: bool,
}

#[derive(serde::Serialize, Clone)]
struct StatusView {
    installed: bool,
    running: bool,
    autostart: bool,
    local_version: String,
    latest_version: Option<String>,
    latest_published: String,
    /// 已安装且不存在更新（true 时前端隐藏「更新」按钮）
    up_to_date: bool,
    bin_path: String,
}

#[derive(serde::Serialize, Clone)]
struct ProgramStatusView {
    id: String,
    name: String,
    repo: String,
    hidden: bool,
    status: StatusView,
}

impl StatusView {
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
            up_to_date,
            bin_path: bin_path.display().to_string(),
        }
    }

    /// 本地即时渲染阶段：未知最新版本，up_to_date 置 false
    /// 避免尚未刷新就把「更新」按钮置灰隐藏
    fn from_local(s: &shared::ProgramStatus, bin_path: &PathBuf, autostart: bool) -> Self {
        Self {
            installed: s.installed,
            running: s.running,
            autostart,
            local_version: s.local_version.clone(),
            latest_version: None,
            latest_published: String::new(),
            up_to_date: false,
            bin_path: bin_path.display().to_string(),
        }
    }
}

#[derive(serde::Deserialize)]
struct StartPayload {
    program_id: String,
    values: BTreeMap<String, String>,
}

fn to_view(p: &Program) -> ProgramView {
    ProgramView {
        id: p.id.clone(),
        name: p.name.clone(),
        description: p.description.clone(),
        repo: p.repo.clone(),
        binary: p.binary.clone(),
        args: p.args.clone(),
        fields: p
            .fields
            .iter()
            .map(|f| to_field_view(f))
            .collect(),
        hidden: p.hidden,
    }
}

fn to_field_view(f: &Field) -> FieldView {
    let (kind, label, default) = match &f.kind {
        shared::config::FieldKind::String { label, default, .. } => {
            ("string", label.clone(), default.clone())
        }
        shared::config::FieldKind::File { label, default, .. } => {
            ("file", label.clone(), default.clone())
        }
        shared::config::FieldKind::Directory { label, default, .. } => {
            ("directory", label.clone(), default.clone())
        }
        shared::config::FieldKind::Boolean { label, default } => {
            ("boolean", label.clone(), default.to_string())
        }
        shared::config::FieldKind::AutoStart { label, default } => {
            ("autostart", label.clone(), default.to_string())
        }
    };
    FieldView { key: f.key.clone(), kind: kind.to_string(), label, default }
}

#[tauri::command]
fn get_programs(state: State<AppState>) -> Vec<ProgramView> {
    let mgr = state.manager.lock().unwrap();
    mgr.programs.iter().map(to_view).collect()
}

/// 用当前网络设置(加速前缀 + 通用代理)构建 GitHub 客户端
fn proxied_github(proxy: &shared::ProxySettings) -> shared::GitHub {
    let mut gh = shared::GitHub::default();
    gh.apply_network(&proxy.accelerate_prefix, &proxy.http_proxy);
    gh
}

#[tauri::command]
fn get_values(state: State<AppState>, program_id: String) -> BTreeMap<String, String> {
    let mgr = state.manager.lock().unwrap();
    let Some(p) = mgr.programs.iter().find(|p| p.id == program_id) else {
        return BTreeMap::new();
    };
    mgr.load_field_values(p)
}

#[tauri::command]
fn save_values(
    state: State<AppState>,
    program_id: String,
    values: BTreeMap<String, String>,
) -> Result<(), String> {
    let mgr = state.manager.lock().unwrap();
    let Some(p) = mgr.programs.iter().find(|p| p.id == program_id) else {
        return Err("程序不存在".into());
    };
    mgr.save_field_values(p, &values);
    Ok(())
}

#[tauri::command]
fn get_status(state: State<AppState>, program_id: String) -> Result<StatusView, String> {
    // 锁内只取本地状态（含 running/autostart），不触网；网络查询放锁外，避免阻塞其它命令
    let p;
    let bin;
    let mut local;
    let autostart;
    {
        let mut mgr = state.manager.lock().unwrap();
        let Some(found) = mgr.programs.iter().find(|p| p.id == program_id).cloned() else {
            return Err("程序不存在".into());
        };
        p = found;
        bin = mgr.bin_path(&p);
        local = mgr.status_local(&p);
        autostart = mgr.autostart.is_enabled(&p.id);
        let layout = mgr.proxy.clone();
        let gh = proxied_github(&layout);
        drop(mgr);
        if let Ok(latest) = gh.latest(&p.repo) {
            local.latest_version = Some(latest.tag_name.trim_start_matches('v').to_string());
            local.latest_published = latest.published_at.clone();
        }
    }
    Ok(StatusView::from_status(&local, &bin, autostart))
}

/// 批量管理：仅本地状态（无网络），用于即时渲染表格第一帧
#[tauri::command]
fn batch_status_local(state: State<AppState>) -> Result<Vec<ProgramStatusView>, String> {
    let mut mgr = state.manager.lock().unwrap();
    let progs = mgr.programs.clone();
    Ok(progs
        .into_iter()
        .map(|p| {
            let bin = mgr.bin_path(&p);
            let s = mgr.status_local(&p);
            let auto = mgr.autostart.is_enabled(&p.id);
            ProgramStatusView {
                id: p.id.clone(),
                name: p.name.clone(),
                repo: p.repo.clone(),
                hidden: p.hidden,
                status: StatusView::from_local(&s, &bin, auto),
            }
        })
        .collect())
}

/// 批量管理：锁内取本地状态、锁外并行查最新版本（网络），最后合并返回
#[tauri::command]
fn batch_status(state: State<AppState>) -> Result<Vec<ProgramStatusView>, String> {
    let layout;
    let mut locals: Vec<(Program, PathBuf, shared::ProgramStatus, bool)> = {
        let mut mgr = state.manager.lock().unwrap();
        let progs = mgr.programs.clone();
        layout = mgr.proxy.clone();
        progs
            .into_iter()
            .map(|p| {
                let bin = mgr.bin_path(&p);
                let s = mgr.status_local(&p);
                let auto = mgr.autostart.is_enabled(&p.id);
                (p, bin, s, auto)
            })
            .collect()
    };
    // 锁外并行：每个 repo 各起一个线程查最新版本（全局缓存避免重复网络）
    let latest: Vec<Option<(String, String)>> = std::thread::scope(|s| {
        let handles: Vec<_> = locals
            .iter()
            .map(|(p, _, _, _)| {
                let repo = p.repo.clone();
                let layout = layout.clone();
                s.spawn(move || {
                    let gh = proxied_github(&layout);
                    gh.latest(&repo)
                        .ok()
                        .map(|r| (r.tag_name.trim_start_matches('v').to_string(), r.published_at.clone()))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or(None))
            .collect()
    });
    for ((_, _, s, _), lv) in locals.iter_mut().zip(latest) {
        if let Some((v, pb)) = lv {
            s.latest_version = Some(v);
            s.latest_published = pb;
        }
    }
    Ok(locals
        .into_iter()
        .map(|(p, bin, s, auto)| ProgramStatusView {
            id: p.id.clone(),
            name: p.name.clone(),
            repo: p.repo.clone(),
            hidden: p.hidden,
            status: StatusView::from_status(&s, &bin, auto),
        })
        .collect())
}

#[tauri::command]
fn install(state: State<AppState>, program_id: String) -> Result<String, String> {
    let mgr = state.manager.lock().unwrap();
    let Some(p) = mgr.programs.iter().find(|p| p.id == program_id).cloned() else {
        return Err("程序不存在".into());
    };
    let data_dir = mgr.data_dir.clone();
    drop(mgr);
    // 独立下载，避免长时间持有锁
    ShellManager::install_standalone(&data_dir, &p).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn start_program(
    state: State<AppState>,
    payload: StartPayload,
) -> Result<StatusView, String> {
    let mut mgr = state.manager.lock().unwrap();
    let Some(p) = mgr.programs.iter().find(|p| p.id == payload.program_id).cloned() else {
        return Err("程序不存在".into());
    };
    mgr.save_field_values(&p, &payload.values);
    // 应用开机启动字段（若配置了）
    if let Err(e) = mgr.apply_key_autostart(&p, &payload.values) {
        log::info!("autostart 设置失败: {e:#}");
    }
    mgr.start(&p, &payload.values).map_err(|e| format!("{e:#}"))?;
    let bin = mgr.bin_path(&p);
    let s = mgr.status(&p);
    Ok(StatusView::from_status(&s, &bin, mgr.autostart.is_enabled(&p.id)))
}

#[tauri::command]
fn stop_program(state: State<AppState>, program_id: String) -> Result<StatusView, String> {
    let mut mgr = state.manager.lock().unwrap();
    let Some(p) = mgr.programs.iter().find(|p| p.id == program_id).cloned() else {
        return Err("程序不存在".into());
    };
    mgr.stop(&program_id).map_err(|e| format!("{e:#}"))?;
    let bin = mgr.bin_path(&p);
    let s = mgr.status(&p);
    Ok(StatusView::from_status(&s, &bin, mgr.autostart.is_enabled(&p.id)))
}

/// 重启：停止后重新加载字段值并启动
#[tauri::command]
fn restart_program(
    state: State<AppState>,
    payload: StartPayload,
) -> Result<StatusView, String> {
    let mut mgr = state.manager.lock().unwrap();
    let Some(p) = mgr.programs.iter().find(|p| p.id == payload.program_id).cloned() else {
        return Err("程序不存在".into());
    };
    mgr.stop(&p.id).map_err(|e| format!("{e:#}"))?;
    mgr.save_field_values(&p, &payload.values);
    if let Err(e) = mgr.apply_key_autostart(&p, &payload.values) {
        log::info!("autostart 设置失败: {e:#}");
    }
    mgr.start(&p, &payload.values).map_err(|e| format!("{e:#}"))?;
    let bin = mgr.bin_path(&p);
    let s = mgr.status(&p);
    Ok(StatusView::from_status(&s, &bin, mgr.autostart.is_enabled(&p.id)))
}

#[tauri::command]
fn stop_all(state: State<AppState>) -> Result<(), String> {
    let mut mgr = state.manager.lock().unwrap();
    mgr.stop_all();
    Ok(())
}

#[tauri::command]
fn set_autostart(
    state: State<AppState>,
    program_id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut mgr = state.manager.lock().unwrap();
    let Some(p) = mgr.programs.iter().find(|p| p.id == program_id).cloned() else {
        return Err("程序不存在".into());
    };
    let bin = mgr.bin_path(&p);
    mgr.autostart
        .set_enabled(&program_id, &bin, enabled)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn reveal_logs(state: State<AppState>, program_id: String) -> Result<(), String> {
    let mgr = state.manager.lock().unwrap();
    let _ = program_id;
    let log_dir = mgr.data_dir.join("logs");
    std::fs::create_dir_all(&log_dir).map_err(|e| format!("{e:#}"))?;
    open_in_file_manager(log_dir)
}

// ---------- 本地实例管理（编辑/删除/隐藏/日志） ----------

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct ProxyView {
    accelerate_prefix: String,
    http_proxy: String,
}

/// 读取当前网络代理/加速设置
#[tauri::command]
fn get_proxy(state: State<AppState>) -> ProxyView {
    let mgr = state.manager.lock().unwrap();
    ProxyView {
        accelerate_prefix: mgr.proxy.accelerate_prefix.clone(),
        http_proxy: mgr.proxy.http_proxy.clone(),
    }
}

/// 保存网络代理/加速设置：持久化到 shell.json 并立即应用到请求客户端。
/// 同时清空通用/版本缓存，使新代理对后续请求即时生效。
#[tauri::command]
fn set_proxy(
    state: State<AppState>,
    accelerate_prefix: String,
    http_proxy: String,
) -> Result<(), String> {
    let mut mgr = state.manager.lock().unwrap();
    mgr.proxy.accelerate_prefix = accelerate_prefix.trim().to_string();
    mgr.proxy.http_proxy = http_proxy.trim().to_string();
    // 应用到 GitHub 客户端（版本查询/下载）——先复制值再避免借用冲突
    let (acc, hp) = (mgr.proxy.accelerate_prefix.clone(), mgr.proxy.http_proxy.clone());
    mgr.github.apply_network(&acc, &hp);
    // 清空全局最新版本缓存，避免旧网络结果残留
    shared::clear_github_cache();
    // 清空注册表本地缓存，使清单/模板走新代理重新拉取
    let reg_cache = mgr.data_dir.join("cache/registry");
    let _ = std::fs::remove_dir_all(&reg_cache);
    mgr.save_config(&state.config_path)
        .map_err(|e| format!("{e:#}"))?;
    Ok(())
}

#[derive(serde::Serialize)]
struct LogsView {
    out: String,
    err: String,
}

/// 读取某程序的最新日志（stdout / stderr 各取尾段）
#[tauri::command]
fn get_logs(state: State<AppState>, program_id: String) -> Result<LogsView, String> {
    let mgr = state.manager.lock().unwrap();
    if !mgr.programs.iter().any(|p| p.id == program_id) {
        return Err("程序不存在".into());
    }
    let (out, err) = mgr.read_logs(&program_id, 64 * 1024);
    Ok(LogsView { out, err })
}

#[derive(serde::Deserialize)]
struct EditField {
    key: String,
    kind: String,
    label: String,
    default: String,
}

/// 编辑程序的完整定义（name/description/repo/binary/args/fields）。
/// 资产规则与架构映射沿用原实例（当前简单 UI 不编辑这两项）。
#[derive(serde::Deserialize)]
struct EditProgramPayload {
    id: String,
    name: String,
    description: String,
    repo: String,
    binary: String,
    args: Vec<String>,
    fields: Vec<EditField>,
}

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
                    placeholder: String::new(),
                },
            };
            shared::config::Field {
                key: f.key.clone(),
                kind,
            }
        })
        .collect();
    Program {
        id: e.id.clone(),
        name: e.name.clone(),
        description: e.description.clone(),
        category: base.category.clone(),
        repo: e.repo.clone(),
        binary: if e.binary.is_empty() {
            e.id.clone()
        } else {
            e.binary.clone()
        },
        assets: base.assets.clone(),
        arch_map: base.arch_map.clone(),
        os_map: base.os_map.clone(),
        fields,
        args: e.args.clone(),
        working_dir: base.working_dir.clone(),
        template_source: base.template_source.clone(),
        imported_at: base.imported_at,
        check_sha256: base.check_sha256.clone(),
        hidden: base.hidden,
    }
}

#[tauri::command]
fn edit_program(
    state: State<AppState>,
    payload: EditProgramPayload,
) -> Result<ProgramView, String> {
    let mut mgr = state.manager.lock().unwrap();
    let Some(base) = mgr.programs.iter().find(|p| p.id == payload.id).cloned() else {
        return Err("程序不存在".into());
    };
    let updated = build_program_from_edit(&payload, &base);
    mgr.update_program(&payload.id, &updated, &state.config_path)
        .map_err(|e| format!("{e:#}"))?;
    let view = to_view(&updated);
    Ok(view)
}

#[tauri::command]
fn delete_program(state: State<AppState>, program_id: String) -> Result<(), String> {
    let mut mgr = state.manager.lock().unwrap();
    mgr.delete_program(&program_id, &state.config_path)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn set_program_hidden(
    state: State<AppState>,
    program_id: String,
    hidden: bool,
) -> Result<(), String> {
    let mut mgr = state.manager.lock().unwrap();
    mgr.set_hidden(&program_id, hidden, &state.config_path)
        .map_err(|e| format!("{e:#}"))
}

// ---------- 模板库 ----------

#[derive(serde::Serialize)]
struct ManifestView {
    revision: String,
    categories: Vec<String>,
    templates: Vec<shared::TemplateIndex>,
    offline: bool,
}

#[tauri::command]
fn get_registries(state: State<AppState>) -> Result<Vec<String>, String> {
    let mgr = state.manager.lock().unwrap();
    Ok(mgr.template_registries.clone())
}

#[tauri::command]
fn get_manifest(
    state: State<AppState>,
    registry_url: String,
) -> Result<ManifestView, String> {
    let mgr = state.manager.lock().unwrap();
    let cache = mgr.data_dir.join("cache/registry");
    let client = shared::RegistryClient::with_network(
        &registry_url,
        cache,
        mgr.registry_pubkeys.clone(),
        Some(&mgr.proxy.accelerate_prefix),
        Some(&mgr.proxy.http_proxy),
    );
    let (offline, manifest) = client
        .load_manifest()
        .map_err(|e| format!("{e:#}"))?;
    Ok(ManifestView {
        revision: manifest.revision,
        categories: manifest.categories,
        templates: manifest.templates,
        offline,
    })
}

/// C5：合并配置里所有注册表，一次性返回(含各源离线/冲突信息)。
#[derive(serde::Serialize)]
struct MergedManifestView {
    templates: Vec<(String, shared::TemplateIndex, String)>, // (base, index)
    sources: Vec<(String, bool)>,
    conflicts: Vec<(String, usize)>, // id -> 提供它的源数量
}

#[tauri::command]
fn get_merged_manifest(state: State<AppState>, registry_url: String) -> Result<MergedManifestView, String> {
    let mgr = state.manager.lock().unwrap();
    let cache = mgr.data_dir.join("cache/registry");
    let mut bases: Vec<String> = mgr.template_registries.clone();
    let typed = registry_url.trim().to_string();
    if !typed.is_empty() && !bases.contains(&typed) {
        bases.push(typed);
    }
    if bases.is_empty() {
        return Err("未配置注册表".into());
    }
    let merged = shared::load_merged_manifests(
        &bases,
        cache,
        mgr.registry_pubkeys.clone(),
        Some(&mgr.proxy.accelerate_prefix),
        Some(&mgr.proxy.http_proxy),
    );
    Ok(MergedManifestView {
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
    })
}

/// 导入：拉取模板 → 快照进本地配置 → 写回 shell.json
#[tauri::command]
fn import_template(
    state: State<AppState>,
    registry_url: String,
    template_id: String,
) -> Result<ProgramView, String> {
    let mut mgr = state.manager.lock().unwrap();
    let cache = mgr.data_dir.join("cache/registry");
    let client = shared::RegistryClient::with_network(
        &registry_url,
        cache,
        mgr.registry_pubkeys.clone(),
        Some(&mgr.proxy.accelerate_prefix),
        Some(&mgr.proxy.http_proxy),
    );
    let (_offline, mut program) = client
        .load_template(&template_id)
        .map_err(|e| format!("{e:#}"))?;

    if mgr.programs.iter().any(|p| p.id == program.id) {
        return Err(format!("程序「{}」已存在", program.id));
    }
    // 允许模板没有显式 binary 时用 id 兜底
    if program.binary.is_empty() {
        program.binary = program.id.clone();
    }
    let view = to_view(&program);
    mgr.programs.push(program);
    mgr.save_config(&state.config_path)
        .map_err(|e| format!("{e:#}"))?;
    Ok(view)
}

#[cfg(target_os = "macos")]
fn open_in_file_manager(path: PathBuf) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{e:#}"))
}

#[cfg(target_os = "linux")]
fn open_in_file_manager(path: PathBuf) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{e:#}"))
}

#[cfg(target_os = "windows")]
fn open_in_file_manager(path: PathBuf) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{e:#}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let data_dir = dirs::data_dir()
        .map(|d| d.join("universal-shell"))
        .unwrap_or_else(|| PathBuf::from("."));
    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join("shell.json"));

    let mut manager = ShellManager::new(data_dir.clone()).expect("初始化数据目录失败");
    if config_path.exists() {
        manager
            .load_config(&config_path)
            .expect("加载配置失败");
    } else if let Ok(cwd) = std::env::current_dir() {
        for cand in [cwd.join("shell.json"), cwd.join("demo/shell.json")] {
            if cand.exists() {
                manager.load_config(&cand).unwrap();
                break;
            }
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            manager: Mutex::new(manager),
            config_path,
        })
        // D1: 托盘常驻。关闭窗口→隐藏而非退出；托盘菜单唤出/退出。
        .setup(|app| {
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

            let show_i = MenuItem::with_id(app, "tray_show", "显示主窗口", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "tray_quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

            let tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "tray_quit" => {
                        app.exit(0);
                    }
                    "tray_show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                });
            tray.build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关闭窗口 → 隐藏到托盘（托盘菜单“退出”才真正结束进程）
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_programs,
            get_values,
            save_values,
            get_status,
            batch_status_local,
            install,
            start_program,
            stop_program,
            restart_program,
            stop_all,
            batch_status,
            set_autostart,
            reveal_logs,
            get_logs,
            edit_program,
            delete_program,
            set_program_hidden,
            get_registries,
            get_manifest,
            get_merged_manifest,
            import_template,
            get_proxy,
            set_proxy
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}