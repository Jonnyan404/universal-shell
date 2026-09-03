//! universal-shell (egui 版)
//!
//! 配置驱动的「二进制程序管理壳」。UI 字段由 shell.json 的 fields 定义驱动：
//! string=文本框 / file=文件选择(rfd) / directory=目录选择(rfd) /
//! boolean=复选框 / autostart=开机启动(立即生效)。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use eframe::egui;
use rust_i18n::t;
use shared::config::FieldKind;
use shared::{RegistryClient, ShellManager};

rust_i18n::i18n!("../shared/locales");

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let data_dir = dirs::data_dir()
        .map(|d| d.join("universal-shell"))
        .unwrap_or_else(|| PathBuf::from("."));
    let config_arg = std::env::args().nth(1);
    let config_path = config_arg
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join("shell.json"));

    let mut manager = ShellManager::new(data_dir.clone())
        .with_context(|| t!("err.init_datadir"))
        .unwrap();

    if config_path.exists() {
        manager.load_config(&config_path).unwrap();
    } else if let Ok(cwd) = std::env::current_dir() {
        // 就近找一个 shell.json 演示配置
        for cand in [cwd.join("shell.json"), cwd.join("demo/shell.json")] {
            if cand.exists() {
                manager.load_config(&cand).unwrap();
                break;
            }
        }
    }

    // 语言：手动覆盖(auto=跟随系统) + 系统提示 解析出最终语言并同步到 rust_i18n
    let override_locale = if manager.locale == "auto" {
        None
    } else {
        Some(manager.locale.as_str())
    };
    shared::locale::apply(override_locale, &system_hint());

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([820.0, 560.0])
            .with_min_inner_size([640.0, 420.0]),
        // glow (OpenGL) 后端，避免 wgpu 在无 GPU/远程会话下 Device lost 崩溃
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Universal Shell (egui)",
        options,
        Box::new(move |cc| {
            install_cjk_font(&cc.egui_ctx);
            let mut app = ShellApp::new(manager, config_path);
            app.install_tray(cc.egui_ctx.clone());
            Ok(Box::new(app))
        }),
    )
}

enum Msg {
    /// 后台安装/更新完成
    InstallDone(String, Option<String>, Option<String>),
    /// 后台安装/更新进度
    InstallProgress(String, shared::DownloadProgress),
    /// 模板库清单加载完成（多源合并结果）
    ManifestLoaded(Result<shared::MergedSource, String>),
    /// 模板拉取完成，携带解析后的 Program 与是否覆盖（供 UI 线程快照进本地配置）
    TemplateFetched(String, bool, Result<shared::config::Program, String>),
    /// 模板更新检查完成：携带 (程序 id, 远端模板, diff 摘要)
    TemplateUpdateChecked(String, Result<shared::config::Program, String>),
    /// 后台刷新各程序最新版本完成：携带 (id, 最新版本, 发布时间)
    StatusRefreshed(Vec<(String, Option<String>, String)>),
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    Manage,
    Library,
    Batch,
}

struct ShellApp {
    manager: ShellManager,
    /// program id -> 表单字段运行时值
    values: BTreeMap<String, BTreeMap<String, String>>,
    /// program id -> 提示消息
    notice: BTreeMap<String, String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    busy: bool,
    /// 当前配置路径（导入模板快照需要写回）
    config_path: PathBuf,
    /// 当前选中的受管程序 id
    current_id: Option<String>,
    // ---- 模板库 ----
    view: View,
    /// 选中的 registry URL
    registry_url: String,
    /// 已加载的多源合并清单
    merged: Option<shared::MergedSource>,
    manifest_offline: bool,
    registry_wait: bool,
    /// 首帧是否已触发过一次清单加载（启动即读离线缓存/联网，对齐 Tauri ensureLibraryFromCache）
    manifest_initialized: bool,
    search: String,
    /// 模板库按来源过滤；None 表示「全部源」
    lib_source: Option<String>,
    /// 模板库分页（当前页，0 起）
    lib_page: usize,
    /// 本地模板抽屉是否展开
    show_local_drawer: bool,
    /// 待二次确认覆盖导入的远端模板（程序 id, base url）
    pending_import: Option<(String, String)>,
    /// 待二次确认覆盖导入的本地模板文件
    pending_local_import: Option<(std::path::PathBuf, shared::config::Program)>,
    /// 模板源管理弹窗是否打开
    show_sources: bool,
    /// 模板源管理：可编辑的源列表（第 0 项为默认官方源）
    sources_rows: Vec<String>,
    /// 模板源管理：新增源输入框内容
    sources_new: String,
    /// 正在导入的模板 id -> 状态
    imports: BTreeMap<String, String>,
    /// 正在检查模板更新的程序 id -> 状态文案
    update_checks: BTreeMap<String, String>,
    /// 已拉到待应用的远端模板(程序 id -> (远端模板, diff 摘要))
    pending_updates: BTreeMap<String, (shared::config::Program, shared::TemplateDiff)>,
    /// 当前下载进度：(程序 id, 完成比例 0.0..=1.0, 阶段文案)
    progress: Option<(String, f64, String)>,
    /// 设置面板：加速前缀 / 通用代理 编辑框
    settings_accel: String,
    settings_proxy: String,
    /// 程序日志查看器是否打开
    show_log: bool,
    /// 壳操作日志弹窗是否打开
    show_shell_log: bool,
    /// 全局设置弹窗是否打开
    show_settings: bool,
    /// 暗色主题
    dark_mode: bool,
    /// 会话内操作日志（本程序页底部滚动条显示，最多保留 200 条）
    op_logs: Vec<String>,
    /// 各程序的最新版本缓存（后台异步刷新，避免渲染时联网卡顿）
    /// key = 程序 id, value = (最新版本, 发布时间)
    latest_versions: BTreeMap<String, (Option<String>, String)>,
    /// 待二次确认删除的程序 id（批量页「删除」按钮触发）
    confirm_delete: Option<String>,
    /// 批量页「检查更新」是否进行中（按钮置灰 + 显示检查中）
    checking_updates: bool,
    /// 最近一次「检查更新」完成时间（unix 秒），用于「上次检查更新」提示
    latest_checked_at: Option<i64>,
    /// 托盘图标（保持存活）
    tray: Option<tray_icon::TrayIcon>,
    /// 是否已在托盘菜单里点了「退出」
    quit: Arc<AtomicBool>,
}

impl ShellApp {
    fn new(manager: ShellManager, config_path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut info_for_values = vec![];
        let mut values = BTreeMap::new();
        for p in &manager.programs {
            info_for_values.push((p.id.clone(), manager.load_field_values(p)));
        }
        for (id, v) in info_for_values {
            values.insert(id, v);
        }
        let mut notice = BTreeMap::new();
        for p in &manager.programs {
            notice.insert(p.id.clone(), String::new());
        }
        let registry_url = manager
            .template_registries
            .first()
            .cloned()
            .unwrap_or_default();
        let current_id = manager.programs.first().map(|p| p.id.clone());
        let settings_accel = manager.proxy.accelerate_prefix.clone();
        let settings_proxy = manager.proxy.http_proxy.clone();
        let sources_rows = manager.template_registries.clone();
        Self {
            manager,
            values,
            notice,
            tx,
            rx,
            busy: false,
            config_path,
            current_id,
            view: View::Manage,
            registry_url,
            merged: None,
            manifest_offline: false,
            registry_wait: false,
            manifest_initialized: false,            search: String::new(),
            lib_source: None,
            lib_page: 0,
            show_local_drawer: false,
            pending_import: None,
            pending_local_import: None,
            show_sources: false,
            sources_rows,
            sources_new: String::new(),
            imports: BTreeMap::new(),
            update_checks: BTreeMap::new(),
            pending_updates: BTreeMap::new(),
            progress: None,
            settings_accel,
            settings_proxy,
            show_log: false,
            show_shell_log: false,
            show_settings: false,
            dark_mode: true,
            op_logs: Vec::new(),
            latest_versions: BTreeMap::new(),
            confirm_delete: None,
            checking_updates: false,
            latest_checked_at: None,
            tray: None,
            quit: Arc::new(AtomicBool::new(false)),
        }
    }

    /// D1: 创建托盘图标（macOS 要求主线程创建，此处在 eframe AppCreator 内调用）。
    /// 菜单：显示主窗口 / 退出；左键单击托盘唤出窗口；点窗口关闭按钮→隐藏到托盘。
    fn install_tray(&mut self, ctx: eframe::egui::Context) {
        let tray = match self.build_tray() {
            Some(t) => t,
            None => {
                log::warn!("创建托盘图标失败，应用将保持普通窗口模式");
                return;
            }
        };
        self.tray = Some(tray);
        let quit = self.quit.clone();
        std::thread::spawn(move || loop {
            // 菜单事件
            while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
                match event.id().as_ref() {
                    "tray_show" => {
                        let _ = ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        let _ = ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    "tray_quit" => {
                        quit.store(true, Ordering::SeqCst);
                        let _ = ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    _ => {}
                }
            }
            // 左键单击托盘 → 唤出
            while let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
                if let tray_icon::TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                } = event
                {
                    let _ = ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    let _ = ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        });
    }

    fn build_tray(&self) -> Option<tray_icon::TrayIcon> {
        use tray_icon::menu::{Menu, MenuItem};
        use tray_icon::{Icon, TrayIconBuilder};

        let show = MenuItem::with_id("tray_show", t!("tray.show"), true, None);
        let quit = MenuItem::with_id("tray_quit", t!("tray.quit"), true, None);
        let menu = Menu::with_items(&[&show, &quit]).ok()?;

        // 做一个 32×32 的纯色图标（避免引入 image 依赖）
        let icon = Icon::from_rgba(solid_icon_rgba(), 32, 32).ok()?;

        TrayIconBuilder::new()
            .with_tooltip("Universal Shell (egui)")
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .build()
            .ok()
    }

    /// 后台线程执行真正下载/解压(数据目录 + 程序是自包含的)
    fn spawn_install(&mut self, program: shared::config::Program) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.progress = Some((
            program.id.clone(),
            0.0,
            t!("dl.downloading").to_string(),
        ));
        let data_dir = self.manager.data_dir.clone();
        let tx = self.tx.clone();
        let pid = program.id.clone();
        std::thread::spawn(move || {
            let on_progress = |p: &shared::DownloadProgress| {
                let _ = tx.send(Msg::InstallProgress(pid.clone(), p.clone()));
            };
            let result = ShellManager::install_standalone_with_progress(&data_dir, &program, &on_progress);
            match result {
                Ok(version) => tx.send(Msg::InstallDone(pid.clone(), Some(version), None)).ok(),
                Err(e) => tx.send(Msg::InstallDone(pid.clone(), None, Some(format!("{e:#}")))).ok(),
            };
        });
        let _ = pid;
    }

    /// 后台拉取清单(合并配置里的所有注册表；load_manifest 自带缓存/离线回退)
    fn spawn_load_manifest(&mut self) {
        if self.registry_wait {
            return;
        }
        // 源列表：既支持配置里的多注册表，也支持临时输入的单个 URL
        let mut bases: Vec<String> = self.manager.template_registries.clone();
        let typed = self.registry_url.trim().to_string();
        if !typed.is_empty() && !bases.contains(&typed) {
            bases.push(typed);
        }
        if bases.is_empty() {
            self.notice
                .insert("__registry__".into(), t!("err.registry_not_configured").to_string());
            return;
        }
        self.registry_wait = true;
        let cache = self.manager.data_dir.join("cache/registry");
        let pubkeys = self.manager.registry_pubkeys.clone();
        let proxy = self.manager.proxy.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let merged = shared::load_merged_manifests(
                &bases,
                cache,
                pubkeys,
                Some(&proxy.accelerate_prefix),
                Some(&proxy.http_proxy),
            );
            tx.send(Msg::ManifestLoaded(Ok(merged))).ok();
        });
    }

    /// 后台拉取模板；成功后由 UI 线程按 `overwrite` 快照进本地配置
    fn spawn_import_template(&mut self, id: String, url: String, overwrite: bool) {
        self.imports.insert(id.clone(), t!("lib.importing").to_string());
        let cache = self.manager.data_dir.join("cache/registry");
        let pubkeys = self.manager.registry_pubkeys.clone();
        let proxy = self.manager.proxy.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let client = RegistryClient::with_network(
                &url,
                cache,
                pubkeys,
                Some(&proxy.accelerate_prefix),
                Some(&proxy.http_proxy),
            );
            match client.load_template(&id) {
                Ok((_, program)) => {
                    tx.send(Msg::TemplateFetched(id.clone(), overwrite, Ok(program))).ok();
                }
                Err(e) => {
                    tx.send(Msg::TemplateFetched(id.clone(), overwrite, Err(format!("{e:#}")))).ok();
                }
            }
        });
    }

    /// 由 template_source 反解出所属注册表 base URL。
    /// 约定来源存为 `<base><id>`（见 registry.load_template）。
    fn registry_base_from_source(&self, program: &shared::config::Program) -> Option<String> {
        let src = program.template_source.as_deref()?;
        let id = &program.id;
        src.strip_suffix(id)
            .filter(|b| b.starts_with("http"))
            .map(|b| b.to_string())
    }

    /// 后台拉取该程序来源注册表里的最新模板，UI 线程据此展示 diff / 应用更新
    fn spawn_check_template_update(&mut self, program: shared::config::Program) {
        if self.update_checks.contains_key(&program.id) {
            return;
        }
        let Some(base) = self.registry_base_from_source(&program) else {
            self.notice
                .insert(program.id.clone(), t!("eg.no_source_registry").to_string());
            return;
        };
        self.update_checks.insert(program.id.clone(), t!("dl.checking_short").to_string());
        let id = program.id.clone();
        let cache = self.manager.data_dir.join("cache/registry");
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let client = RegistryClient::new(&base, cache);
            match client.load_template(&id) {
                Ok((_, remote)) => {
                    tx.send(Msg::TemplateUpdateChecked(id.clone(), Ok(remote))).ok();
                }
                Err(e) => {
                    tx.send(Msg::TemplateUpdateChecked(id.clone(), Err(format!("{e:#}")))).ok();
                }
            }
        });
    }

    fn handle_msgs(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::InstallProgress(pid, p) => {
                    let label = match p.stage {
                        shared::DownloadStage::Downloading => t!("dl.progress", pct = (p.fraction().unwrap_or(0.0) * 100.0).round() as u64).to_string(),
                        shared::DownloadStage::Verifying => t!("dl.verifying").to_string(),
                        shared::DownloadStage::Extracting => t!("dl.extracting").to_string(),
                    };
                    let frac = p.fraction().unwrap_or(0.0);
                    self.progress = Some((pid, frac, label));
                }
                Msg::InstallDone(pid, version, err) => {
                    self.busy = false;
                    self.progress = None;
                    let text = match version {
                        Some(v) => t!("toast.updated_short", ver = v).to_string(),
                        None => t!("toast.download_fail", err = err.unwrap_or_default()).to_string(),
                    };
                    self.notice.insert(pid, text);
                }
                Msg::ManifestLoaded(result) => {
                    self.registry_wait = false;
                    match result {
                        Ok(merged) => {
                            self.merged = Some(merged.clone());
                            self.manifest_offline =
                                merged.sources.iter().any(|(_, off, _)| *off);
                            let n = merged.sources.len();
                            let msg = if self.manifest_offline {
                                t!("eg.loaded_offline", n = n)
                            } else {
                                t!("eg.loaded", n = n)
                            }
                            .to_string();
                            self.notice.insert("__registry__".into(), msg);
                        }
                        Err(e) => {
                            self.merged = None;
                            self.notice
                                .insert("__registry__".into(), t!("toast.manifest_fail", err = e).to_string());
                        }
                    }
                }
                Msg::TemplateFetched(id, overwrite, result) => {
                    match result {
                        Ok(program) => {
                            // 快照到本地配置：追加/覆盖程序 + 写回 shell.json
                            if let Err(e) = self.commit_import(&program, overwrite) {
                                self.imports.insert(
                                    id.clone(),
                                    t!("eg.import_fail", err = format!("{e:#}")).to_string(),
                                );
                                return;
                            }
                            self.imports.remove(&id);
                            self.notice.insert(
                                "__registry__".into(),
                                t!("eg.imported_snap", id = id, path = self.config_path.display())
                                    .to_string(),
                            );
                        }
                                Err(e) => {
                                    self.imports.insert(id.clone(), t!("eg.import_fail", err = e).to_string());
                                }
                    }
                }
                Msg::TemplateUpdateChecked(pid, result) => {
                    self.update_checks.remove(&pid);
                    match result {
                        Ok(remote) => {
                            let cur = self.shown_program().cloned();
                            let cur = cur.as_ref().or_else(|| {
                                self.manager.programs.iter().find(|p| p.id == pid).clone()
                            });
                            let Some(cur) = cur else {
                                self.notice.insert(pid.clone(), t!("eg.not_found_instance").to_string());
                                return;
                            };
                            let diff = self.manager.template_diff(cur, &remote);
                            if diff.is_empty() {
                                self.pending_updates.remove(&pid);
                                self.notice
                                    .insert(pid.clone(), t!("eg.no_diff").to_string());
                            } else {
                                self.pending_updates.insert(pid.clone(), (remote, diff));
                                let summary = self
                                    .pending_updates
                                    .get(&pid)
                                    .map(|(_, d)| d.summary())
                                    .unwrap_or_default();
                                self.notice
                                    .insert(pid.clone(), t!("eg.update_found", summary = summary).to_string());
                            }
                        }
                        Err(e) => {
                            self.notice.insert(pid.clone(), t!("eg.check_update_fail", err = e).to_string());
                        }
                    }
                }
                Msg::StatusRefreshed(list) => {
                    for (id, ver, ts) in list {
                        self.latest_versions.insert(id, (ver, ts));
                    }
                    self.checking_updates = false;
                    self.latest_checked_at = Some(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0),
                    );
                    self.notice.insert("__batch__".into(), t!("dl.done").to_string());
                }
            }
        }
    }

    fn refresh_status(&mut self) {
        // 后台异步刷新各程序最新版本（联网，避免主线程卡顿）
        let programs = self.manager.programs.clone();
        let proxy = self.manager.proxy.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut gh = shared::GitHub::default();
            gh.apply_network(&proxy.accelerate_prefix, &proxy.http_proxy);
            let mut out = Vec::with_capacity(programs.len());
            for p in &programs {
                let latest = gh
                    .latest(&p.repo)
                    .map(|l| (Some(l.tag_name.trim_start_matches('v').to_string()), l.published_at))
                    .unwrap_or((None, String::new()));
                out.push((p.id.clone(), latest.0, latest.1));
            }
            let _ = tx.send(Msg::StatusRefreshed(out));
        });
    }

    /// 渲染用状态：本地状态 + 后台缓存的最新版本（不在渲染时联网）。
    fn display_status(&mut self, p: &shared::config::Program) -> shared::ProgramStatus {
        let mut st = self.manager.status_local(p);
        if let Some((v, ts)) = self.latest_versions.get(&p.id) {
            st.latest_version = v.clone();
            st.latest_published = ts.clone();
        }
        st
    }

    /// 应用远端模板更新到实例，保留用户已填字段值后写回配置
    fn apply_pending_update(&mut self, cur: shared::config::Program, remote: shared::config::Program) {
        let mut next = cur.clone();
        let values = self.values.get(&cur.id).cloned().unwrap_or_default();
        let merged = shared::ShellManager::apply_template_update(&mut next, &remote, &values);
        // 用新字段值刷新表单
        self.values.insert(cur.id.clone(), merged);
        // 替换本地配置并写盘
        if let Some(slot) = self.manager.programs.iter_mut().find(|p| p.id == cur.id) {
            *slot = next;
        }
        match self.manager.save_config(&self.config_path) {
            Ok(()) => {
                self.notice.insert(
                    cur.id.clone(),
                    t!("eg.updated_written", path = self.config_path.display()).to_string(),
                );
            }
            Err(e) => {
                self.notice.insert(cur.id.clone(), t!("eg.write_fail", err = format!("{e:#}")).to_string());
            }
        }
        self.pending_updates.remove(&cur.id);
    }

    /// 把拉取到的模板追加进本地配置并写回 disk（快照）；overwrite=true 时覆盖同名程序
    fn commit_import(&mut self, program: &shared::config::Program, overwrite: bool) -> anyhow::Result<()> {
        if let Some(idx) = self.manager.programs.iter().position(|p| p.id == program.id) {
            if !overwrite {
                anyhow::bail!(t!("err.program_exists", id = program.id));
            }
            self.manager.programs[idx] = program.clone();
        } else {
            // 写入用户运行时值（默认值）
            let defaults = self.manager.load_field_values(program);
            self.values.insert(program.id.clone(), defaults);
            self.notice
                .entry(program.id.clone())
                .or_default();
            if self.current_id.is_none() {
                self.current_id = Some(program.id.clone());
            }
            self.manager.programs.push(program.clone());
        }
        self.manager.save_config(&self.config_path)
    }

    fn shown_program(&self) -> Option<&shared::config::Program> {
        let id = self.current_id.as_deref()?;
        self.manager.programs.iter().find(|p| p.id == id)
    }

    fn show_form(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.shown_program().cloned() else {
            return;
        };
        let pid = p.id.clone();
        let values = self.values.entry(pid.clone()).or_default();

        for field in &p.fields {
            // 开机启动统一由批量管理页管理，程序页不再显示该字段
            if matches!(field.kind, FieldKind::AutoStart { .. }) {
                continue;
            }
            match &field.kind {
                FieldKind::String { label, placeholder, .. } => {
                    ui.horizontal(|ui| {
                        let v = values.entry(field.key.clone()).or_default();
                        let empty = v.is_empty();
                        ui.add_sized([140.0, 0.0], required_label(field.required, empty, label));
                        ui.add(
                            egui::TextEdit::singleline(v)
                                .hint_text(placeholder)
                                .desired_width(f32::INFINITY),
                        );
                    });
                }
                FieldKind::File { label, filter, .. } => {
                    ui.horizontal(|ui| {
                        let v = values.entry(field.key.clone()).or_default();
                        let empty = v.is_empty();
                        ui.add_sized([140.0, 0.0], required_label(field.required, empty, label));
                        ui.add(egui::TextEdit::singleline(v).desired_width(f32::INFINITY));
                        if ui.button(t!("act.browse")).clicked() {
                            let mut dlg = rfd::FileDialog::new();
                            if !filter.is_empty() {
                                let patterns: Vec<&str> =
                                    filter.split(',').map(|s| s.trim()).collect();
                                dlg = dlg.add_filter("文件", &patterns);
                            }
                            if let Some(path) = dlg.pick_file() {
                                *v = path.display().to_string();
                            }
                        }
                    });
                }
                FieldKind::Directory { label, .. } => {
                    ui.horizontal(|ui| {
                        let v = values.entry(field.key.clone()).or_default();
                        let empty = v.is_empty();
                        ui.add_sized([140.0, 0.0], required_label(field.required, empty, label));
                        ui.add(egui::TextEdit::singleline(v).desired_width(f32::INFINITY));
                        if ui.button(t!("act.browse")).clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                *v = path.display().to_string();
                            }
                        }
                    });
                }
                FieldKind::Boolean { label, .. } => {
                    let v = values.entry(field.key.clone()).or_default();
                    let mut b = v == "true";
                    if ui.checkbox(&mut b, label).changed() {
                        *v = if b { "true" } else { "false" }.to_string();
                    }
                }
                FieldKind::AutoStart { label, .. } => {
                    let v = values.entry(field.key.clone()).or_default();
                    let mut b = v == "true";
                    if ui.checkbox(&mut b, label).changed() {
                        *v = if b { "true" } else { "false" }.to_string();
                        // 立即应用开机启动
                        let snapshot = values.clone();
                        let target = self
                            .manager
                            .programs
                            .iter()
                            .find(|p| p.id == pid.clone())
                            .cloned();
                        if let Some(p) = target {
                            if let Err(e) = self.manager.apply_key_autostart(&p, &snapshot) {
                                self.notice.insert(pid.clone(), t!("eg.autostart_fail", err = format!("{e:#}")).to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    /// 左侧栏：主题/语言/设置按钮 + 程序列表 + 批量/模板库/壳日志链接
    fn show_sidebar(&mut self, ui: &mut egui::Ui) {
        // 顶部按钮行：主题 / 语言 / 设置
        ui.horizontal(|ui| {
            let theme_label = if self.dark_mode { "☀" } else { "☾" };
            let theme_hint = if self.dark_mode { t!("ui.theme.light") } else { t!("ui.theme.dark") };
            if ui.small_button(theme_label).on_hover_text(theme_hint).clicked() {
                self.dark_mode = !self.dark_mode;
                let visuals = if self.dark_mode {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                };
                ui.ctx().set_visuals(visuals);
            }
            let lang_label = match self.manager.locale.as_str() {
                "auto" => "文",
                "zh-CN" => "中",
                _ => "EN",
            };
            if ui.small_button(lang_label).on_hover_text(t!("ui.lang")).clicked() {
                let next = match self.manager.locale.as_str() {
                    "auto" => "zh-CN",
                    "zh-CN" => "en",
                    _ => "auto",
                };
                self.manager.locale = next.to_string();
                let override_locale = if next == "auto" { None } else { Some(next) };
                shared::locale::apply(override_locale, &system_hint());
                let _ = self.manager.save_config(&self.config_path);
            }
            if ui.small_button("⚙").on_hover_text(t!("ui.settings")).clicked() {
                self.show_settings = true;
            }
        });
        ui.separator();

        // 程序列表（可滚动），预留底部链接空间
        let programs = self.manager.programs.clone();
        let visible: Vec<_> = programs.iter().filter(|p| !p.hidden).collect();
        let selected_id = self.current_id.clone();
        let list_height = (ui.available_height() - 86.0).max(80.0);
        egui::ScrollArea::vertical()
            .id_salt("sidebar_scroll")
            .auto_shrink([false, false])
            .max_height(list_height)
            .show(ui, |ui| {
                if visible.is_empty() {
                    ui.weak(format!(
                        "{}{}",
                        t!("side.empty_hint"),
                        t!("side.empty_suffix")
                    ));
                    return;
                }
                for p in visible {
                    let active = selected_id.as_deref() == Some(p.id.as_str());
                    let st = self.manager.status_local(p);
                    let initial = p
                        .name
                        .trim()
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().collect::<String>())
                        .unwrap_or_else(|| "?".into());
                    // 副标题：未安装 / 运行中 / 已停止（对齐 Tauri renderSidebar）
                    let sub = if !st.installed {
                        t!("st.not_installed", repo = p.repo).to_string()
                    } else if st.running {
                        t!("st.running_ver", ver = st.local_version).to_string()
                    } else {
                        t!("st.stopped_ver", ver = st.local_version).to_string()
                    };

                    let response = egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .fill(if active {
                            ui.visuals().selection.bg_fill
                        } else {
                            ui.visuals().faint_bg_color
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                // 状态点：运行绿 / 停止灰
                                ui.colored_label(
                                    if st.running {
                                        egui::Color32::from_rgb(90, 180, 90)
                                    } else {
                                        egui::Color32::from_rgb(150, 150, 150)
                                    },
                                    if st.running { "●" } else { "○" },
                                );
                                // 首字母图标
                                ui.monospace(initial);
                                // 名称 + 副标题
                                ui.vertical(|ui| {
                                    let name_color = if active {
                                        ui.visuals().strong_text_color()
                                    } else {
                                        ui.visuals().text_color()
                                    };
                                    ui.label(
                                        egui::RichText::new(&p.name).color(name_color).strong(),
                                    );
                                    ui.small(sub);
                                });
                            });
                        })
                        .response
                        .interact(egui::Sense::click());
                    if response.clicked() {
                        self.current_id = Some(p.id.clone());
                        self.view = View::Manage;
                    }
                }
            });

        // 底部链接：批量 / 模板库 / 壳日志（对齐 Tauri sidebar 底部 + foot）
        ui.separator();
        if ui.selectable_label(matches!(self.view, View::Batch), t!("batch.title")).clicked() {
            self.view = View::Batch;
        }
        if ui.selectable_label(matches!(self.view, View::Library), t!("lib.title")).clicked() {
            self.view = View::Library;
        }
        ui.separator();
        if ui.selectable_label(matches!(self.view, View::Library), t!("ui.shell_log_short")).clicked() {
            self.show_shell_log = true;
        }
    }

    /// 主内容区：标题栏 + 视图分派
    fn show_main(&mut self, ui: &mut egui::Ui) {
        // 标题栏
        ui.horizontal(|ui| {
            ui.heading(t!("app.name"));
            ui.weak(t!("eg.data_dir", path = self.manager.data_dir.display()));
        });
        ui.separator();

        match self.view {
            View::Manage => self.show_manage(ui),
            View::Library => self.show_library(ui),
            View::Batch => self.show_batch(ui),
        }
    }

    /// 全局设置弹窗
    fn show_settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        egui::Window::new(t!("sett.title"))
            .id(egui::Id::new("settings_window"))
            .open(&mut open)
            .default_width(500.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized([100.0, 0.0], egui::Label::new(t!("sett.accelerate")));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings_accel)
                            .hint_text(t!("sett.acc_placeholder"))
                            .desired_width(f32::INFINITY),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add_sized([100.0, 0.0], egui::Label::new(t!("sett.proxy")));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings_proxy)
                            .hint_text(t!("sett.proxy_placeholder"))
                            .desired_width(f32::INFINITY),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t!("act.save")).clicked() {
                            let accel = self.settings_accel.trim().to_string();
                            let proxy = self.settings_proxy.trim().to_string();
                            self.manager.proxy.accelerate_prefix = accel.clone();
                            self.manager.proxy.http_proxy = proxy.clone();
                            self.manager.github.apply_network(&accel, &proxy);
                            if let Err(e) = self.manager.save_config(&self.config_path) {
                                self.notice.insert("__settings__".into(), format!("{e:#}"));
                            } else {
                                self.notice.insert("__settings__".into(), t!("toast.saved").to_string());
                            }
                        }
                    });
                });
                if let Some(n) = self.notice.get("__settings__") {
                    if !n.is_empty() {
                        ui.colored_label(egui::Color32::from_rgb(90, 180, 90), n);
                    }
                }
            });
        self.show_settings = open;
    }

    fn show_manage(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.shown_program().cloned() else {
            ui.label(t!("eg.no_program"));
            return;
        };

        // 状态行：程序名 + 状态标签 + 下载按钮（匹配 Tauri statusbar）
        let status = self.display_status(&p);
        let up_to_date = status
            .latest_version
            .as_ref()
            .map(|v| *v == status.local_version)
            .unwrap_or(false);
        let show_dl = !(status.installed && (up_to_date || status.latest_version.is_none()));
        ui.horizontal(|ui| {
            ui.heading(&p.name);
            ui.add_space(8.0);
            ui.weak(&p.repo);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if show_dl {
                    let dl_text = if self.busy {
                        t!("dl.downloading").to_string()
                    } else if status.installed {
                        t!("dl.update").to_string()
                    } else {
                        t!("dl.download").to_string()
                    };
                    let dl_btn = ui.add_enabled(!self.busy, egui::Button::new(dl_text));
                    if dl_btn.clicked() {
                        self.spawn_install(p.clone());
                        self.notice.insert(p.id.clone(), String::new());
                    }
                }
            });
        });
        // 状态标签（对齐 Tauri renderStatus chips）
        ui.horizontal_wrapped(|ui| {
            ui.label(t!("st.local_ver", ver = status.local_version));
            if status.installed {
                ui.colored_label(egui::Color32::from_rgb(90, 180, 90), t!("st.installed"));
            } else {
                ui.colored_label(egui::Color32::from_rgb(150, 150, 150), t!("st.not_installed_bare"));
            }
            if self.manager.program_autostart(&p) {
                ui.colored_label(egui::Color32::from_rgb(90, 180, 90), t!("st.autostart"));
            }
            if status.latest_version.is_some() {
                ui.label(t!("eg.latest_ver_fmt", ver = status.latest_version.as_deref().unwrap_or("")));
            }
            if self.manager.runner.is_running(&p.id) {
                ui.colored_label(egui::Color32::from_rgb(90, 180, 90), t!("st.running"));
            } else {
                ui.colored_label(egui::Color32::from_rgb(150, 150, 150), t!("st.stopped"));
            }
        });

        // 模板更新行（仅当实例带来源注册表时显示）
        if p.template_source.is_some() {
            ui.horizontal(|ui| {
                let checking = self.update_checks.get(&p.id).cloned();
                let btn = ui.add_enabled(
                    checking.is_none(),
                    egui::Button::new(
                        if checking.is_some() { t!("dl.checking_short").to_string() } else { t!("eg.check_update").to_string() },
                    ),
                );
                if btn.clicked() {
                    self.spawn_check_template_update(p.clone());
                    self.notice.insert(p.id.clone(), String::new());
                }
                if let Some((_, diff)) = self.pending_updates.get(&p.id).cloned() {
                    let mut detail = diff.changed_fields_detail.clone();
                    ui.label(diff.summary());
                    if ui.button(t!("eg.view_changes")).clicked() {
                        self.notice.insert(
                            p.id.clone(),
                            if detail.is_empty() {
                                diff.summary()
                            } else {
                                detail.drain(..).collect::<Vec<_>>().join("；")
                            },
                        );
                    }
                    if ui.button(t!("eg.apply_update")).clicked() {
                        if let Some((remote, _)) = self.pending_updates.get(&p.id).cloned() {
                            self.apply_pending_update(p.clone(), remote);
                        }
                    }
                }
            });
            ui.add_space(4.0);
        }

        ui.separator();

        // 配置驱动表单（预留下方操作区/日志空间，避免把按钮挤出可视区）
        let form_height = (ui.available_height() - 220.0).max(60.0);
        egui::ScrollArea::vertical()
            .max_height(form_height)
            .show(ui, |ui| {
                self.show_form(ui);
            });

        ui.add_space(8.0);

        // 下载进度条
        if let Some((pid, frac, label)) = self.progress.clone() {
            if pid == p.id {
                ui.label(label);
                ui.add(
                    egui::ProgressBar::new(frac as f32)
                        .show_percentage()
                        .desired_width(ui.available_width()),
                );
                ui.add_space(4.0);
            }
        }

        // 操作区：启动/停止/重启（对齐 Tauri renderActions，处于运行态时停/重启可用、启动禁用）+ 图标按钮
        let values = self.values.get(&p.id).cloned().unwrap_or_default();
        let url = web_url(&p, &values);
        let running = self.manager.runner.is_running(&p.id);
        ui.horizontal(|ui| {
            let values_for_start = values;
            let running_now = running;
            let start_btn = ui.add_enabled(!running_now, egui::Button::new(format!("▶ {}", t!("act.start"))));
            if start_btn.clicked() {
                self.manager.save_field_values(&p, &values_for_start);
                match self.manager.start(&p, &values_for_start) {
                    Ok(()) => {
                        self.log_op(&t!("op.start", name = &p.name));
                        self.notice.insert(p.id.clone(), String::new());
                    }
                    Err(e) => {
                        self.notice.insert(p.id.clone(), t!("toast.start_fail", err = format!("{e:#}")).to_string());
                    }
                }
            }
            let stop_btn = ui.add_enabled(running_now, egui::Button::new(format!("■ {}", t!("act.stop"))));
            if stop_btn.clicked() {
                match self.manager.stop(&p.id) {
                    Ok(()) => {
                        self.log_op(&t!("op.stop", name = &p.name));
                        self.notice.insert(p.id.clone(), String::new());
                    }
                    Err(e) => {
                        self.notice.insert(p.id.clone(), format!("{e:#}"));
                    }
                }
            }
            let restart_btn = ui.add_enabled(running_now, egui::Button::new(format!("↻ {}", t!("act.restart"))));
            if restart_btn.clicked() {
                self.restart_program(&p, &values_for_start);
                self.log_op(&t!("op.restart", name = &p.name));
            }
            ui.separator();
            if ui.button("📁").on_hover_text(t!("act.open_app_dir")).clicked() {
                let app_dir = self.manager.app_dir(&p);
                let _ = std::process::Command::new(&open_cmd()).arg(&app_dir).spawn();
            }
            if url.is_some() {
                if ui.button("⧉").on_hover_text(t!("act.copy_addr")).clicked() {
                    ui.ctx().copy_text(url.as_deref().unwrap().to_string());
                    self.notice.insert(p.id.clone(), t!("toast.addr_copied").to_string());
                }
                if ui.button("↗").on_hover_text(t!("act.open_site")).clicked() {
                    let cmd = open_cmd();
                    let _ = std::process::Command::new(&cmd).arg(url.as_deref().unwrap()).spawn();
                }
            }
        });

        if let Some(n) = self.notice.get(&p.id) {
            if !n.is_empty() {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::from_rgb(200, 90, 90), n);
            }
        }

        // 操作日志条（op-log）
        if !self.op_logs.is_empty() {
            ui.add_space(4.0);
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("op_log_scroll")
                .max_height(90.0)
                .show(ui, |ui| {
                    for line in &self.op_logs {
                        ui.monospace(format!("▪ {line}"));
                    }
                });
        }

        // 内嵌程序日志终端（manage-log，仅运行中显示日志内容）
        self.show_manage_log(ui, &p);
    }

    /// 内嵌程序日志终端：显示当前程序日志（stderr 行以 \x1F 开头标红）。
    fn show_manage_log(&mut self, ui: &mut egui::Ui, p: &shared::config::Program) {
        if !self.manager.runner.is_running(&p.id) {
            return;
        }
        ui.add_space(4.0);
        ui.separator();
        // 内嵌日志操作栏（对齐 Tauri manage-log-actions）
        let mut fullscreen = false;
        ui.horizontal(|ui| {
            if ui.small_button("⛶").on_hover_text(t!("lib.fullscreen")).clicked() {
                fullscreen = true;
            }
            if ui.small_button("⧉").on_hover_text(t!("act.copy")).clicked() {
                let (log, _) = self.manager.read_logs(&p.id, 64 * 1024);
                ui.ctx().copy_text(log);
                self.notice
                    .insert(p.id.clone(), t!("toast.copied").to_string());
            }
            if ui.small_button("↻").on_hover_text(t!("act.refresh")).clicked() {
                ui.ctx().request_repaint();
            }
        });
        if fullscreen {
            self.show_log = true;
        }
        let (log, _) = self.manager.read_logs(&p.id, 64 * 1024);
        egui::ScrollArea::vertical()
            .id_salt("manage_log_scroll")
            .max_height(120.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for line in log.split('\n') {
                    if let Some(rest) = line.strip_prefix('\u{1f}') {
                        ui.colored_label(egui::Color32::from_rgb(220, 90, 90), rest);
                    } else {
                        ui.monospace(line);
                    }
                }
            });
    }

    /// 记录一条会话操作日志（推入 UI 数组 + 持久化到壳日志），最多保留 200 条。
    fn log_op(&mut self, msg: &str) {
        self.manager.log_op(msg);
        self.op_logs.push(msg.to_string());
        if self.op_logs.len() > 200 {
            let over = self.op_logs.len() - 200;
            self.op_logs.drain(..over);
        }
    }

    /// 重启程序：停止 → 保存字段值 → 启动（与 Tauri restart_program 一致）。
    fn restart_program(&mut self, p: &shared::config::Program, values: &BTreeMap<String, String>) {
        if let Err(e) = self.manager.stop(&p.id) {
            self.notice.insert(p.id.clone(), t!("toast.restart_fail", err = format!("{e:#}")).to_string());
            return;
        }
        self.manager.save_field_values(p, values);
        if let Err(e) = self.manager.start(p, values) {
            self.notice.insert(p.id.clone(), t!("toast.restart_fail", err = format!("{e:#}")).to_string());
        } else {
            self.notice.insert(p.id.clone(), t!("toast.restarted").to_string());
        }
    }

    fn show_library(&mut self, ui: &mut egui::Ui) {
        const LIB_PAGE_SIZE: usize = 12;
        // 工具栏：标题 + 本地模板切换 + 导入本地模板（对齐 Tauri library-view 工具栏）
        ui.horizontal(|ui| {
            ui.heading(t!("lib.title"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t!("lib.sources")).clicked() {
                    self.open_sources();
                }
                if ui.button(t!("lib.import")).clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                        self.import_local_path(path);
                    }
                }
                if ui.button(format!("{} ({})", t!("lib.local"), self.manager.programs.len())).clicked() {
                    self.show_local_drawer = !self.show_local_drawer;
                }
            });
        });
        ui.separator();

        // 本地模板抽屉
        if self.show_local_drawer {
            self.show_local_drawer_ui(ui);
        }

        let merged = self.merged.clone();
        // 源栏：来源下拉 + 状态 + 刷新（对齐 Tauri renderSourceBar）
        self.show_source_bar(ui, merged.as_ref());

        // 状态摘要（对齐 Tauri lib-status）
        match &merged {
            Some(m) => {
                let n_off = m.sources.iter().filter(|(_, off, _)| *off).count();
                let summary = if m.sources.is_empty() {
                    t!("lib.no_sources").to_string()
                } else {
                    t!(
                        "lib.summary",
                        n = m.sources.len(),
                        offline = if n_off > 0 {
                            t!("lib.summary_offline", n = n_off).to_string()
                        } else {
                            String::new()
                        },
                        m = m.template_count()
                    )
                    .to_string()
                };
                ui.label(summary);
            }
            None => {
                ui.label(t!("lib.empty_remote"));
            }
        }
        ui.separator();

        // 搜索框（对齐 Tauri lib-search，占位文案）
        ui.horizontal(|ui| {
            ui.label(t!("eg.search"));
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text(t!("lib.search_ph"))
                    .desired_width(240.0),
            );
        });
        ui.add_space(6.0);

        let Some(merged) = merged else {
            return;
        };

        // 过滤 + 分页
        let keyword = self.search.trim().to_lowercase();
        let mut rows: Vec<(String, shared::TemplateIndex, String)> = Vec::new();
        for (id, (base, t)) in &merged.by_id {
            if let Some(src) = &self.lib_source {
                if base != src {
                    continue;
                }
            }
            if !keyword.is_empty() {
                let hay = format!(
                    "{} {} {} {}{}",
                    id, t.name, t.category, t.description,
                    if t.repo.is_empty() { "" } else { &t.repo }
                )
                .to_lowercase();
                if !hay.contains(&keyword) {
                    continue;
                }
            }
            rows.push((id.clone(), t.clone(), base.clone()));
        }
        let pages = usize::max(1, rows.len().div_ceil(LIB_PAGE_SIZE));
        if self.lib_page >= pages {
            self.lib_page = pages - 1;
        }
        let start = self.lib_page * LIB_PAGE_SIZE;
        if start >= rows.len() {
            ui.label(t!("lib.no_match"));
        } else {
            let end = usize::min(start + LIB_PAGE_SIZE, rows.len());
            let slice_owned = rows.drain(start..end).collect::<Vec<_>>();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (id, t, base) in slice_owned {
                        self.render_lib_card(ui, id, t, base, &merged);
                    }
                });
        }

        // 分页（对齐 Tauri renderPager）
        if pages > 1 {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let prev = ui.add_enabled(
                    self.lib_page > 0,
                    egui::Button::new(t!("lib.prev")),
                );
                if prev.clicked() {
                    self.lib_page -= 1;
                }
                ui.label(format!("{} / {}", self.lib_page + 1, pages));
                let next = ui.add_enabled(
                    self.lib_page + 1 < pages,
                    egui::Button::new(t!("lib.next")),
                );
                if next.clicked() {
                    self.lib_page += 1;
                }
            });
        }
    }

    /// 模板库：渲染单个模板卡片（名称/[类别]/仓库/冲突徽标 + 导入按钮 + 描述 + 已导入徽标）
    fn render_lib_card(
        &mut self,
        ui: &mut egui::Ui,
        id: String,
        t: shared::TemplateIndex,
        base: String,
        merged: &shared::MergedSource,
    ) {
        let imported = self.manager.programs.iter().any(|p| p.id == id);
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(&t.name);
                    ui.label(format!("[{}]", t.category));
                    ui.weak(&t.repo);
                    let conflicts = merged.id_conflicts(&id);
                    if conflicts.len() > 1 {
                        ui.colored_label(
                            egui::Color32::from_rgb(200, 150, 50),
                            t!("lib.multi_source", n = conflicts.len()),
                        );
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            let import_state = self.imports.get(&id).cloned();
                            match import_state {
                                None => {
                                    if ui.button(t!("act.import")).clicked() {
                                        if imported {
                                            self.pending_import = Some((id.clone(), base.clone()));
                                        } else {
                                            self.spawn_import_template(id.clone(), base.clone(), false);
                                        }
                                    }
                                }
                                Some(state) => {
                                    ui.add_enabled(false, egui::Button::new(state));
                                }
                            }
                        },
                    );
                });
                ui.label(&t.description);
                if imported {
                    // 「本地」标识用 ok 绿（对齐 Tauri .lib-local-badge 的 --ok 底色）
                    ui.colored_label(egui::Color32::from_rgb(90, 180, 90), t!("lib.local_last"));
                }
            });
        ui.add_space(6.0);
    }

    /// 源栏：来源下拉（含 offline/cache/timeAgo 状态）+ 刷新按钮 + fetch 信息行
    fn show_source_bar(&mut self, ui: &mut egui::Ui, merged: Option<&shared::MergedSource>) {
        ui.horizontal(|ui| {
            let srcs: Vec<(String, bool, u64)> = if let Some(m) = merged {
                m.sources.clone()
            } else {
                self.manager
                    .template_registries
                    .iter()
                    .map(|r| (r.clone(), false, 0u64))
                    .collect()
            };
            let sel = self.lib_source.clone().unwrap_or_else(|| "__all__".into());
            let sel_key = |b: &str| -> String {
                if b == "__all__" {
                    t!("lib.all_sources").to_string()
                } else {
                    b.to_string()
                }
            };
            let mut chosen = sel.clone();
            egui::ComboBox::from_id_salt("lib_source_combo")
                .selected_text(sel_key(&sel))
                .width(300.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, "__all__".to_string(), sel_key("__all__"));
                    for (base, off, fetched) in &srcs {
                        // 「本地/离线」标识带颜色，其余常规色（多段文本）
                        let mut job = egui::text::LayoutJob::default();
                        if *off {
                            let off_label = t!("lib.offline").to_string();
                            job.append(
                                &off_label,
                                0.0,
                                egui::TextFormat {
                                    color: egui::Color32::from_rgb(200, 150, 50),
                                    ..Default::default()
                                },
                            );
                        }
                        job.append(base, if *off { 4.0 } else { 0.0 }, egui::TextFormat::default());
                        if *fetched != 0 {
                            let ago = format!(" · {}", time_ago(*fetched as i64));
                            job.append(
                                &ago,
                                0.0,
                                egui::TextFormat {
                                    color: egui::Color32::GRAY,
                                    ..Default::default()
                                },
                            );
                        }
                        ui.selectable_value(&mut chosen, base.clone(), job);
                    }
                });
            if chosen != sel {
                self.lib_source = if chosen == "__all__" { None } else { Some(chosen) };
                self.lib_page = 0;
            }
            // 刷新按钮：拉取中置灰 + 显示「拉取中…」（对齐 Tauri）
            let refresh = ui.add_enabled(
                !self.registry_wait,
                egui::Button::new(
                    if self.registry_wait {
                        t!("lib.pull").to_string()
                    } else {
                        t!("act.refresh").to_string()
                    },
                ),
            );
            if refresh.clicked() {
                self.spawn_load_manifest();
            }
        });
        // fetch 信息行（对齐 Tauri lib-fetch-info）
        if let Some(m) = merged {
            if m.sources.is_empty() {
                ui.weak(t!("lib.remote_none"));
            } else {
                let mut parts = Vec::new();
                let mut titles = Vec::new();
                for (base, off, fetched) in &m.sources {
                    titles.push(format!(
                        "{base}{}",
                        if *fetched != 0 {
                            t!("lib.last_pull", date = fmt_datetime(*fetched as i64)).to_string()
                        } else {
                            t!("lib.not_pulled").to_string()
                        }
                    ));
                    let suffix = if *off {
                        if *fetched != 0 {
                            t!("lib.cache").to_string() + &time_ago(*fetched as i64)
                        } else {
                            t!("lib.no_cache").to_string()
                        }
                    } else if *fetched != 0 {
                        t!("log.equal_parts").to_string() + " " + &time_ago(*fetched as i64)
                    } else {
                        t!("log.just").to_string()
                    };
                    parts.push(if *off {
                        t!("lib.offline_status", suffix = suffix).to_string()
                    } else {
                        t!("lib.online_status", suffix = suffix).to_string()
                    });
                }
                let line =
                    t!("log.remote_ready").to_string() + &parts.join("　");
                ui.weak(&line).on_hover_text(titles.join("；"));
            }
        } else {
            ui.weak(t!("lib.remote_sources"));
        }
        ui.add_space(4.0);
    }

    /// 本地模板抽屉：列出已导入程序 + 管理按钮（对齐 Tauri renderLocalTemplates）
    fn show_local_drawer_ui(&mut self, ui: &mut egui::Ui) {
        ui.strong(t!("lib.local_has"));
        let list = self.manager.programs.clone();
        if list.is_empty() {
            ui.label(t!("lib.empty_local"));
            ui.separator();
            return;
        }
        for p in &list {
            ui.horizontal(|ui| {
                ui.strong(&p.name);
                ui.weak(&p.repo);
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button(t!("act.manage")).clicked() {
                            self.current_id = Some(p.id.clone());
                            self.view = View::Manage;
                        }
                    },
                );
            });
        }
        ui.separator();
    }

    /// 读取本地 JSON 模板文件并解析为 Program；已存在时排队二次确认覆盖
    fn import_local_path(&mut self, path: std::path::PathBuf) {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.notice.insert(
                    "__registry__".into(),
                    t!("err.read_template_fail", err = e.to_string()).to_string(),
                );
                return;
            }
        };
        let mut program: shared::config::Program = match serde_json::from_str(&text) {
            Ok(p) => p,
            Err(e) => {
                self.notice.insert(
                    "__registry__".into(),
                    t!("err.parse_template_fail", err = e.to_string()).to_string(),
                );
                return;
            }
        };
        if program.binary.is_empty() {
            program.binary = program.id.clone();
        }
        let exists = self
            .manager
            .programs
            .iter()
            .any(|p| p.id == program.id);
        if exists {
            self.pending_local_import = Some((path, program));
        } else {
            match self.commit_import(&program, false) {
                Ok(()) => {
                    self.notice
                        .insert("__registry__".into(), t!("lib.imported_local").to_string());
                }
                Err(e) => {
                    self.notice
                        .insert("__registry__".into(), format!("{e:#}"));
                }
            }
        }
    }

    /// 模板库覆盖导入确认弹窗（远端模板 / 本地文件）
    fn show_library_confirms(&mut self, ctx: &egui::Context) {
        if let Some((id, base)) = self.pending_import.clone() {
            let mut open = true;
            let mut confirmed = false;
            let mut cancelled = false;
            egui::Window::new(t!("lib.overwrite"))
                .id(egui::Id::new("import_confirm"))
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(t!("confirm.overwrite_import", id = &id));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(t!("act.cancel")).clicked() {
                            cancelled = true;
                        }
                        if ui.button(t!("act.import")).clicked() {
                            confirmed = true;
                        }
                    });
                });
            if cancelled || confirmed || !open {
                self.pending_import = None;
            }
            if confirmed {
                self.spawn_import_template(id.clone(), base.clone(), true);
            }
        }
        if let Some((_path, program)) = self.pending_local_import.clone() {
            let mut open = true;
            let mut confirmed = false;
            let mut cancelled = false;
            egui::Window::new(t!("lib.overwrite"))
                .id(egui::Id::new("local_import_confirm"))
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(t!("lib.already_exists"));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(t!("act.cancel")).clicked() {
                            cancelled = true;
                        }
                        if ui.button(t!("act.import")).clicked() {
                            confirmed = true;
                        }
                    });
                });
            if cancelled || confirmed || !open {
                self.pending_local_import = None;
            }
            if confirmed {
                match self.commit_import(&program, true) {
                    Ok(()) => {
                        self.notice
                            .insert("__registry__".into(), t!("lib.imported_local").to_string());
                    }
                    Err(e) => {
                        self.notice
                            .insert("__registry__".into(), format!("{e:#}"));
                    }
                }
            }
        }
    }

    /// 打开模板源管理弹窗（对齐 Tauri openSourcesModal/renderSourcesList）
    fn open_sources(&mut self) {
        let rows = self.manager.template_registries.clone();
        self.sources_rows = if rows.is_empty() {
            vec![String::new()]
        } else {
            rows
        };
        self.sources_new = String::new();
        self.show_sources = true;
    }

    /// 保存模板源（对齐 Tauri saveSources/set_registries）；返回是否成功
    fn save_sources(&mut self) -> bool {
        let cleaned: Vec<String> = self
            .sources_rows
            .iter()
            .map(|s| {
                let s = s.trim().to_string();
                if s.ends_with('/') {
                    s
                } else {
                    format!("{s}/")
                }
            })
            .filter(|s| !s.is_empty() && s != "/")
            .collect();
        if cleaned.is_empty() {
            self.notice.insert("__sources__".into(), t!("lib.keep_one").to_string());
            return false;
        }
        self.manager.template_registries = cleaned.clone();
        if let Err(e) = self.manager.save_config(&self.config_path) {
            self.notice.insert("__sources__".into(), format!("{e:#}"));
            return false;
        }
        self.manager.log_op(&t!("op.update_sources", list = cleaned.join(", ")));
        self.registry_url = cleaned.first().cloned().unwrap_or_default();
        self.lib_source = cleaned.first().cloned();
        self.lib_page = 0;
        // 源已变更：清空清单缓存，回到「尚未拉取」状态，待用户刷新重拉
        self.merged = None;
        self.show_sources = false;
        self.notice.insert("__registry__".into(), t!("toast.sources_saved").to_string());
        true
    }

    /// 模板源管理弹窗（对齐 Tauri sources-modal）
    fn show_sources_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_sources;
        let mut cancel = false;
        let mut save = false;
        egui::Window::new(t!("sources.title"))
            .id(egui::Id::new("sources_window"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_min_width(440.0);
                let mut remove: Option<usize> = None;
                for (i, row) in self.sources_rows.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(row)
                                .hint_text(t!("sources.placeholder"))
                                .desired_width(320.0),
                        );
                        if i == 0 {
                            ui.label(t!("lib.default_rule"));
                        } else if ui
                            .small_button("✕")
                            .on_hover_text(t!("lib.delete_source"))
                            .clicked()
                        {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    self.sources_rows.remove(i);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.sources_new)
                            .hint_text(t!("sources.placeholder"))
                            .desired_width(320.0),
                    );
                    if ui.button(t!("act.add")).clicked() {
                        let v = self.sources_new.trim().to_string();
                        if !v.is_empty() {
                            self.sources_rows.push(v);
                            self.sources_new = String::new();
                        }
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui.button(t!("act.cancel")).clicked() {
                                cancel = true;
                            }
                            if ui.button(t!("act.save")).clicked() {
                                save = true;
                            }
                        },
                    );
                });
            });
        if cancel || !open {
            self.show_sources = false;
        }
        if save {
            self.save_sources();
        }
    }

    /// 批量管理视图：列出所有受管程序，每行 start/stop + 打开应用目录，统一刷新/停止。
    fn show_batch(&mut self, ui: &mut egui::Ui) {
        let programs = self.manager.programs.clone();
        // 工具栏：刷新状态 / 检查更新 / 停止所有（匹配 Tauri batch-toolbar）
        ui.horizontal(|ui| {
            ui.heading(t!("batch.title"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t!("batch.refresh")).clicked() {
                    // 仅重读本地状态（不含网络请求，避免卡顿）
                    ui.ctx().request_repaint();
                }
                // 检查更新：检查中置灰 + 显示「检查中…」，完成恢复（对齐 Tauri checkUpdates）
                if self.checking_updates {
                    ui.add_enabled(false, egui::Button::new(t!("dl.checking_short")));
                } else if ui.button(t!("batch.check")).clicked() {
                    self.checking_updates = true;
                    self.notice.insert("__batch__".into(), t!("dl.checking").to_string());
                    self.refresh_status();
                    ui.ctx().request_repaint();
                }
                if ui.button(t!("batch.stop_all")).clicked() {
                    self.manager.stop_all();
                    self.log_op(&t!("op.stop_all"));
                }
            });
        });
        // 「上次检查更新」提示（对齐 Tauri #batch-checked-at）
        ui.horizontal(|ui| {
            ui.weak(if let Some(at) = self.latest_checked_at {
                t!("check.last_checked", ago = time_ago(at)).to_string()
            } else {
                t!("check.not_checked").to_string()
            });
        });
        ui.separator();

        if programs.is_empty() {
            ui.label(t!("eg.no_program"));
            return;
        }

        egui::ScrollArea::both().show(ui, |ui| {
            // 表头（固定列宽对齐信息行）
            ui.horizontal(|ui| {
                ui.add_sized(
                    [160.0, 0.0],
                    egui::Label::new(egui::RichText::new(t!("th.program")).strong()),
                );
                ui.add_sized(
                    [80.0, 0.0],
                    egui::Label::new(egui::RichText::new(t!("th.local_ver")).strong()),
                );
                ui.add_sized(
                    [80.0, 0.0],
                    egui::Label::new(egui::RichText::new(t!("th.latest_ver")).strong()),
                );
                ui.add_sized(
                    [90.0, 0.0],
                    egui::Label::new(egui::RichText::new(t!("th.status")).strong()),
                );
                ui.strong(t!("th.autostart"));
                ui.strong(t!("th.hidden"));
            });
            ui.separator();

            for p in &programs {
                let st = self.display_status(p);
                // 第一行：程序信息
                ui.horizontal(|ui| {
                    ui.add_sized([160.0, 0.0], egui::Label::new(&p.name));
                    if p.hidden {
                        ui.colored_label(egui::Color32::from_rgb(150, 150, 150), t!("st.hidden"));
                    }
                    ui.add_sized(
                        [80.0, 0.0],
                        egui::Label::new(if st.installed {
                            st.local_version.clone()
                        } else {
                            t!("st.not_installed_bare").to_string()
                        }),
                    );
                    ui.add_sized(
                        [80.0, 0.0],
                        egui::Label::new(
                            st.latest_version.clone().unwrap_or_else(|| t!("st.unknown").to_string()),
                        ),
                    );
                    if !st.installed {
                        ui.add_sized(
                            [90.0, 0.0],
                            egui::Label::new(
                                egui::RichText::new(t!("st.not_installed_bare"))
                                    .color(egui::Color32::from_rgb(150, 150, 150)),
                            ),
                        );
                    } else if st.running {
                        ui.add_sized(
                            [90.0, 0.0],
                            egui::Label::new(
                                egui::RichText::new(t!("st.running"))
                                    .color(egui::Color32::from_rgb(90, 180, 90)),
                            ),
                        );
                    } else {
                        ui.add_sized(
                            [90.0, 0.0],
                            egui::Label::new(
                                egui::RichText::new(t!("st.stopped"))
                                    .color(egui::Color32::from_rgb(150, 150, 150)),
                            ),
                        );
                    }
                    let mut auto = self.manager.program_autostart(p);
                    if ui.checkbox(&mut auto, "").changed() {
                        self.set_autostart(p, auto);
                    }
                    let mut hidden = p.hidden;
                    if ui.checkbox(&mut hidden, "").changed() {
                        match self.manager.set_hidden(&p.id, hidden, &self.config_path) {
                            Ok(()) => {
                                self.log_op(&t!(
                                    "op.toggle_visibility",
                                    showhide = t!(if hidden { "op.hide" } else { "op.show" }),
                                    name = &p.name
                                ));
                            }
                            Err(e) => {
                                self.notice.insert(p.id.clone(), format!("{e:#}"));
                            }
                        }
                    }
                });
                // 第二行：操作按钮横向铺开
                let up_to_date = st.installed
                    && st.latest_version.as_deref() == Some(st.local_version.as_str());
                let dl_label = if !st.installed {
                    t!("dl.download").to_string()
                } else if up_to_date {
                    t!("st.latest").to_string()
                } else {
                    t!("dl.update").to_string()
                };
                ui.horizontal(|ui| {
                    ui.label(t!("th.actions"));
                    ui.add_space(4.0);
                    let dl_btn = ui.add_enabled(!up_to_date, egui::Button::new(dl_label));
                    if dl_btn.clicked() {
                        self.spawn_install(p.clone());
                    }
                    if st.running {
                        let restart = ui.small_button(t!("act.restart"));
                        if restart.clicked() {
                            let values = self.values.get(&p.id).cloned().unwrap_or_default();
                            self.restart_program(p, &values);
                            self.log_op(&t!("op.restart", name = &p.name));
                        }
                        if ui.small_button(t!("act.stop")).clicked() {
                            if let Ok(()) = self.manager.stop(&p.id) {
                                self.log_op(&t!("op.stop", name = &p.name));
                            }
                        }
                    } else if ui.small_button(t!("act.start")).clicked() {
                        let values = self.values.get(&p.id).cloned().unwrap_or_default();
                        self.manager.save_field_values(p, &values);
                        if let Ok(()) = self.manager.start(p, &values) {
                            self.log_op(&t!("op.start", name = &p.name));
                        }
                    }
                    if ui.small_button(t!("act.log")).clicked() {
                        self.current_id = Some(p.id.clone());
                        self.show_log = true;
                    }
                    if ui.small_button(t!("act.open_app_dir")).on_hover_text(t!("act.open_app_dir")).clicked() {
                        let d = self.manager.app_dir(p);
                        let _ = std::process::Command::new(&open_cmd()).arg(&d).spawn();
                    }
                    if ui.small_button(t!("act.manage")).clicked() {
                        self.current_id = Some(p.id.clone());
                        self.view = View::Manage;
                    }
                    if ui.small_button(t!("act.delete")).clicked() {
                        self.confirm_delete = Some(p.id.clone());
                    }
                });
                ui.separator();
            }
        });

        if let Some(n) = self.notice.get("__batch__") {
            if !n.is_empty() {
                ui.add_space(4.0);
                ui.weak(n);
            }
        }
    }

    /// 切换程序开机自启（写入该程序 AutoStart 字段值持久化）。
    fn set_autostart(&mut self, p: &shared::config::Program, enabled: bool) {
        let key = p
            .fields
            .iter()
            .find(|f| matches!(f.kind, FieldKind::AutoStart { .. }))
            .map(|f| f.key.clone());
        if let Some(key) = key {
            let mut values = self.manager.load_field_values(p);
            values.insert(key, if enabled { "true".into() } else { "false".into() });
            self.manager.save_field_values(p, &values);
            self.log_op(&t!(
                "op.toggle_autostart",
                onoff = t!(if enabled { "op.enable" } else { "op.disable" }),
                name = &p.name
            ));
        }
    }

    /// 程序日志查看器：浮窗展示当前程序日志尾部（对齐 Tauri log-modal，含复制/刷新/打开日志目录/关闭）。
    fn show_log_window(&mut self, ctx: &egui::Context) {
        let Some(p) = self.shown_program().cloned() else {
            self.show_log = false;
            return;
        };
        let title = t!("log.title_fmt", name = p.name).to_string();
        let mut open = self.show_log;
        let mut copied = false;
        let mut close_clicked = false;
        let log_msg: String = t!("st.empty").to_string();
        egui::Window::new(title)
            .id(egui::Id::new("program_log_window"))
            .open(&mut open)
            .default_size([640.0, 360.0])
            .show(ctx, |ui| {
                // 操作栏：复制 / 刷新 / 打开日志目录 / 关闭
                ui.horizontal(|ui| {
                    if ui.small_button("⧉").on_hover_text(t!("act.copy")).clicked() {
                        let (log, _) = self.manager.read_logs(&p.id, 64 * 1024);
                        ui.ctx().copy_text(log);
                        copied = true;
                    }
                    if ui.small_button("↻").on_hover_text(t!("act.refresh")).clicked() {
                        ctx.request_repaint();
                    }
                    if ui.small_button("📁").on_hover_text(t!("act.open_log_dir")).clicked() {
                        let d = self.manager.data_dir.join("logs");
                        let _ = std::process::Command::new(&open_cmd()).arg(&d).spawn();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text(t!("act.close")).clicked() {
                            close_clicked = true;
                        }
                    });
                });
                ui.separator();
                // 日志内容（stderr 行以 \x1F 开头标红）
                let (log, _) = self.manager.read_logs(&p.id, 64 * 1024);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in log.split('\n') {
                            if let Some(rest) = line.strip_prefix('\u{1f}') {
                                ui.colored_label(egui::Color32::from_rgb(220, 90, 90), rest);
                            } else {
                                ui.monospace(line);
                            }
                        }
                        if log.is_empty() {
                            ui.weak(log_msg);
                        }
                    });
            });
        if close_clicked {
            open = false;
        }
        self.show_log = open;
        if copied {
            self.notice
                .insert("__log__".into(), t!("toast.copied").to_string());
        }
    }

    /// 壳操作日志弹窗：显示 shell.log 内容（对齐 Tauri shell-log-modal，含刷新/清空/关闭）。
    fn show_shell_log_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_shell_log;
        let mut clear_req = false;
        let mut close_clicked = false;
        egui::Window::new(t!("shell_log.title"))
            .id(egui::Id::new("shell_log_window"))
            .open(&mut open)
            .default_size([560.0, 320.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.weak(t!("shell_log.hint"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text(t!("act.close")).clicked() {
                            close_clicked = true;
                        }
                        if ui.small_button("⟳").on_hover_text(t!("act.refresh")).clicked() {
                            ctx.request_repaint();
                        }
                        if ui.small_button("🗑").on_hover_text(t!("act.clear")).clicked() {
                            clear_req = true;
                        }
                    });
                });
                ui.separator();
                let content = std::fs::read_to_string(self.manager.op_log_path())
                    .unwrap_or_default();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if content.trim().is_empty() {
                            ui.weak(t!("shell_log.empty"));
                        } else {
                            for line in content.lines() {
                                ui.monospace(line);
                            }
                        }
                    });
            });
        if close_clicked {
            open = false;
        }
        self.show_shell_log = open;
        if clear_req {
            self.manager.clear_op_log();
        }
    }

    /// 删除确认弹窗：二次确认后真正删除程序并清理数据目录。
    fn show_delete_confirm(&mut self, ctx: &egui::Context, id: &str) {
        let title = t!("act.delete").to_string();
        let mut open = true;
        let mut confirmed = false;
        let mut closing = false;
        let name = self
            .manager
            .programs
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| id.to_string());
        egui::Window::new(title)
            .id(egui::Id::new("delete_confirm"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(t!("confirm.delete", name = name));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(t!("act.cancel")).clicked() {
                        closing = true;
                    }
                    if ui.button(t!("act.delete")).clicked() {
                        confirmed = true;
                        closing = true;
                    }
                });
            });
        if !open || closing {
            self.confirm_delete = None;
        }
        if confirmed {
            if let Some(id) = self.confirm_delete.take() {
                let name = self
                    .manager
                    .programs
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.name.clone());
                match self.manager.delete_program(&id, &self.config_path) {
                    Ok(()) => {
                        // 取消选中/清理相关运行时状态
                        if self.current_id.as_deref() == Some(id.as_str()) {
                            self.current_id = None;
                        }
                        self.values.remove(&id);
                        self.latest_versions.remove(&id);
                        self.notice.insert(
                            "__batch__".into(),
                            t!("toast.deleted", name = name.unwrap_or(id)).to_string(),
                        );
                    }
                    Err(e) => {
                        let msg = format!("{e:#}");
                        self.notice.insert("__batch__".into(), msg);
                    }
                }
            }
        }
    }
}

impl eframe::App for ShellApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_msgs();
        // 检查更新进行中或崩溃恢复/运行态轮询：3s 一次（不发网络请求，仅读本地进程/文件）
        ctx.request_repaint_after(if self.checking_updates {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(3)
        });
        // 关闭窗口 → 隐藏到托盘（托盘「退出」才真正退出）
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.quit.load(Ordering::SeqCst) {
                return;
            }
            let _ = ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            let _ = ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 启动即加载清单（读离线缓存或联网刷新），避免重启后模板库为空、需手动刷新（对齐 Tauri ensureLibraryFromCache+refresh）
        if !self.manifest_initialized {
            self.manifest_initialized = true;
            self.spawn_load_manifest();
        }
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(200.0)
            .show(ui, |ui| {
                self.show_sidebar(ui);
            });
        egui::CentralPanel::default()
            .show(ui, |ui| {
                self.show_main(ui);
            });

        if self.show_log {
            self.show_log_window(ui.ctx());
        }
        if self.show_shell_log {
            self.show_shell_log_window(ui.ctx());
        }
        if self.show_settings {
            self.show_settings_window(ui.ctx());
        }
        if let Some(id) = self.confirm_delete.clone() {
            self.show_delete_confirm(ui.ctx(), &id);
        }
        self.show_library_confirms(ui.ctx());
        if self.show_sources {
            self.show_sources_window(ui.ctx());
        }
    }
}

fn open_cmd() -> String {
    if cfg!(target_os = "macos") {
        "open".into()
    } else if cfg!(target_os = "windows") {
        "explorer".into()
    } else {
        "xdg-open".into()
    }
}

/// 相对时间（对齐 Tauri `timeAgo`）：刚刚 / N 分钟前 / N 小时前 / N 天前 / 绝对日期。
fn time_ago(secs: i64) -> String {
    if secs <= 0 {
        return t!("st.never").to_string();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let diff = (now - secs).max(0);
    if diff < 60 {
        t!("st.just").to_string()
    } else if diff < 3600 {
        t!("time.min_ago", n = diff / 60).to_string()
    } else if diff < 86400 {
        t!("time.hour_ago", n = diff / 3600).to_string()
    } else if diff < 86400 * 30 {
        t!("time.day_ago", n = diff / 86400).to_string()
    } else {
        fmt_datetime(secs)
    }
}

/// 将 unix 秒格式化为 `YYYY-MM-DD HH:MM:SS`（Hinnant 的 civil_from_days，无 chrono 依赖）。
fn fmt_datetime(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let secs_of_day = secs.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        secs_of_day / 3600,
        secs_of_day % 3600 / 60,
        secs_of_day % 60
    )
}

/// 字段标签；必填且当前为空时标红并加红色 `*`。
fn required_label(required: bool, empty: bool, label: &str) -> egui::Label {
    let text = if required {
        format!("{label} *")
    } else {
        label.to_string()
    };
    let rich = if required && empty {
        egui::RichText::new(text).color(egui::Color32::from_rgb(220, 80, 80))
    } else {
        egui::RichText::new(text)
    };
    egui::Label::new(rich)
}

/// 构造程序的 Web 访问地址(如有 host/bind/addr + port 字段)。无地址返回 None。
/// 与 Tauri 前端 `webUrl` 保持一致的取值逻辑。
fn web_url(program: &shared::config::Program, values: &BTreeMap<String, String>) -> Option<String> {
    let field_val = |key: &str| -> Option<String> {
        if let Some(v) = values.get(key) {
            let t = v.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
        program
            .fields
            .iter()
            .find(|f| f.key == key)
            .and_then(|f| {
                let d = f.default_raw();
                let t = d.trim();
                if t.is_empty() { None } else { Some(t.to_string()) }
            })
    };
    let addr = field_val("host")
        .or_else(|| field_val("bind"))
        .or_else(|| field_val("addr"))?;
    if addr.starts_with("http://") || addr.starts_with("https://") {
        return Some(addr);
    }
    let port = field_val("port");
    if let Some(p) = port {
        if !addr.ends_with(&p) && !regex_like_addr(&addr) {
            return Some(format!("http://{addr}:{p}"));
        }
        return Some(format!("http://{addr}:{p}"));
    }
    Some(format!("http://{addr}"))
}

/// 地址是否已含端口(scheme://host:port)。简单判断结尾 :数字。
fn regex_like_addr(addr: &str) -> bool {
    let Some((_, rest)) = addr.split_once(':') else {
        return false;
    };
    rest.chars().all(|c| c.is_ascii_digit())
}

/// 获取系统语言提示(供 `shared::locale::apply` 使用)，失败回退 en。
fn system_hint() -> String {
    sys_locale::get_locale().unwrap_or_else(|| "en".to_string())
}

/// 加载系统中文字体并注入 egui fallback，避免界面中文显示为「口」。
/// 命中顺序：macOS PingFang/Hiragino，Windows 微软雅黑，Linux Noto CJK。
/// 失败仅告警，不阻断启动。
fn install_cjk_font(ctx: &egui::Context) {
    use std::sync::Arc;

    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            "C:\\Windows\\Fonts\\msyh.ttc",
            "C:\\Windows\\Fonts\\msyh.ttf",
            "C:\\Windows\\Fonts\\simhei.ttf",
        ]
    } else {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        ]
    };

    let mut installed = false;
    for path in candidates {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut fonts = egui::FontDefinitions::default();
        fonts
            .font_data
            .insert("cjk".to_string(), Arc::new(egui::FontData::from_owned(bytes)));
        // 把 CJK 字体追加到各 family 的 fallback 链末尾，
        // 让中文字形回退到 "cjk"，拉丁字形仍走默认字体。
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push("cjk".to_string());
            }
        }
        ctx.set_fonts(fonts);
        log::info!("注入中文字体: {path}");
        installed = true;
        break;
    }
    if !installed {
        log::warn!("未找到系统中文字体，界面中文可能显示为「口」(仅影响显示)");
    }
}

/// 生成一个 32×32 的纯色 RGBA 托盘图标（青蓝色圆角方块）
fn solid_icon_rgba() -> Vec<u8> {
    let size = 32u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let edge = x == 0 || y == 0 || x == size - 1 || y == size - 1;
            if edge {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                rgba.extend_from_slice(&[0x20, 0xc9, 0x97, 255]);
            }
        }
    }
    rgba
}