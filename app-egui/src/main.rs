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
use shared::config::FieldKind;
use shared::{RegistryClient, ShellManager};

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
        .with_context(|| "初始化数据目录失败")
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
    /// 模板库清单加载完成（多源合并结果）
    ManifestLoaded(Result<shared::MergedSource, String>),
    /// 模板拉取完成，携带解析后的 Program（供 UI 线程快照进本地配置）
    TemplateFetched(String, Result<shared::config::Program, String>),
    /// 模板更新检查完成：携带 (程序 id, 远端模板, diff 摘要)
    TemplateUpdateChecked(String, Result<shared::config::Program, String>),
}

enum View {
    Manage,
    Library,
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
    search: String,
    /// 正在导入的模板 id -> 状态
    imports: BTreeMap<String, String>,
    /// 正在检查模板更新的程序 id -> 状态文案
    update_checks: BTreeMap<String, String>,
    /// 已拉到待应用的远端模板(程序 id -> (远端模板, diff 摘要))
    pending_updates: BTreeMap<String, (shared::config::Program, shared::TemplateDiff)>,
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
            search: String::new(),
            imports: BTreeMap::new(),
            update_checks: BTreeMap::new(),
            pending_updates: BTreeMap::new(),
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

        let show = MenuItem::with_id("tray_show", "显示主窗口", true, None);
        let quit = MenuItem::with_id("tray_quit", "退出", true, None);
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
        let data_dir = self.manager.data_dir.clone();
        let tx = self.tx.clone();
        let pid = program.id.clone();
        std::thread::spawn(move || {
            let result = ShellManager::install_standalone(&data_dir, &program);
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
                .insert("__registry__".into(), "未配置注册表，请先填写 URL".into());
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

    /// 后台拉取模板；成功后由 UI 线程快照进本地配置
    fn spawn_import_template(&mut self, id: String, url: String) {
        self.imports.insert(id.clone(), "导入中…".into());
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
                    tx.send(Msg::TemplateFetched(id.clone(), Ok(program))).ok();
                }
                Err(e) => {
                    tx.send(Msg::TemplateFetched(id.clone(), Err(format!("{e:#}")))).ok();
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
                .insert(program.id.clone(), "该程序无来源注册表，无法检查更新".into());
            return;
        };
        self.update_checks.insert(program.id.clone(), "检查中…".into());
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
                Msg::InstallDone(pid, version, err) => {
                    self.busy = false;
                    let text = match version {
                        Some(v) => format!("下载/更新完成，当前版本 {v}"),
                        None => format!("下载/更新失败: {}", err.unwrap_or_default()),
                    };
                    self.notice.insert(pid, text);
                    self.refresh_status();
                }
                Msg::ManifestLoaded(result) => {
                    self.registry_wait = false;
                    match result {
                        Ok(merged) => {
                            self.merged = Some(merged.clone());
                            self.manifest_offline =
                                merged.sources.iter().any(|(_, off)| *off);
                            let n = merged.sources.len();
                            let msg = if self.manifest_offline {
                                format!("加载完成（{n} 个源，含离线回退）")
                            } else {
                                format!("加载完成（{n} 个源）")
                            };
                            self.notice.insert("__registry__".into(), msg);
                        }
                        Err(e) => {
                            self.merged = None;
                            self.notice
                                .insert("__registry__".into(), format!("清单拉取失败: {e}"));
                        }
                    }
                }
                Msg::TemplateFetched(id, result) => {
                    match result {
                        Ok(program) => {
                            // 快照到本地配置：追加程序 + 写回 shell.json
                            if let Err(e) = self.commit_import(&program) {
                                self.imports.insert(
                                    id.clone(),
                                    format!("导入失败: {}", format!("{e:#}")),
                                );
                                return;
                            }
                            self.imports.remove(&id);
                            self.notice.insert(
                                "__registry__".into(),
                                format!("已导入模板「{id}」，快照已写入 {}", self.config_path.display()),
                            );
                        }
                        Err(e) => {
                            self.imports.insert(id.clone(), format!("导入失败: {e}"));
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
                                self.notice.insert(pid.clone(), "找不到该程序实例".into());
                                return;
                            };
                            let diff = self.manager.template_diff(cur, &remote);
                            if diff.is_empty() {
                                self.pending_updates.remove(&pid);
                                self.notice
                                    .insert(pid.clone(), "模板已是最新，无差异".into());
                            } else {
                                self.pending_updates.insert(pid.clone(), (remote, diff));
                                let summary = self
                                    .pending_updates
                                    .get(&pid)
                                    .map(|(_, d)| d.summary())
                                    .unwrap_or_default();
                                self.notice
                                    .insert(pid.clone(), format!("发现模板更新: {summary}"));
                            }
                        }
                        Err(e) => {
                            self.notice.insert(pid.clone(), format!("检查模板更新失败: {e}"));
                        }
                    }
                }
            }
        }
    }

    fn refresh_status(&mut self) {
        // 触发一次状态重读：每个程序读本地文件 + GitHub 最新版本
        for p in self.manager.programs.clone() {
            let _ = self.manager.status(&p);
        }
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
                    format!("模板已更新并写回 {}", self.config_path.display()),
                );
            }
            Err(e) => {
                self.notice.insert(cur.id.clone(), format!("写回配置失败: {e:#}"));
            }
        }
        self.pending_updates.remove(&cur.id);
    }

    /// 把拉取到的模板追加进本地配置并写回 disk（快照）
    fn commit_import(&mut self, program: &shared::config::Program) -> anyhow::Result<()> {
        if self
            .manager
            .programs
            .iter()
            .any(|p| p.id == program.id)
        {
            anyhow::bail!("程序「{}」已存在于本地配置", program.id);
        }
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
        self.manager.save_config(&self.config_path)
    }

    fn shown_program(&self) -> Option<&shared::config::Program> {
        let id = self.current_id.as_deref()?;
        self.manager.programs.iter().find(|p| p.id == id)
    }

    /// 渲染受管程序的 Tab 栏（B9：多程序切换）
    fn show_program_tabs(&mut self, ui: &mut egui::Ui) {
        let selected: Vec<String> = self.manager.programs.iter().map(|p| p.id.clone()).collect();
        if selected.is_empty() {
            return;
        }
        ui.horizontal(|ui| {
            for id in &selected {
                let label = self
                    .manager
                    .programs
                    .iter()
                    .find(|p| &p.id == id)
                    .map(|p| p.name.as_str())
                    .unwrap_or(id);
                let active = self.current_id.as_deref() == Some(id.as_str());
                if ui.selectable_label(active, label).clicked() {
                    self.current_id = Some(id.clone());
                }
            }
        });
    }

    fn show_form(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.shown_program().cloned() else {
            return;
        };
        let pid = p.id.clone();
        let values = self.values.entry(pid.clone()).or_default();

        for field in &p.fields {
            match &field.kind {
                FieldKind::String { label, placeholder, .. } => {
                    ui.horizontal(|ui| {
                        ui.add_sized([140.0, 0.0], egui::Label::new(label));
                        let v = values.entry(field.key.clone()).or_default();
                        ui.add(
                            egui::TextEdit::singleline(v)
                                .hint_text(placeholder)
                                .desired_width(f32::INFINITY),
                        );
                    });
                }
                FieldKind::File { label, filter, .. } => {
                    ui.horizontal(|ui| {
                        ui.add_sized([140.0, 0.0], egui::Label::new(label));
                        let v = values.entry(field.key.clone()).or_default();
                        ui.add(egui::TextEdit::singleline(v).desired_width(f32::INFINITY));
                        if ui.button("浏览…").clicked() {
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
                        ui.add_sized([140.0, 0.0], egui::Label::new(label));
                        let v = values.entry(field.key.clone()).or_default();
                        ui.add(egui::TextEdit::singleline(v).desired_width(f32::INFINITY));
                        if ui.button("浏览…").clicked() {
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
                                self.notice.insert(pid.clone(), format!("设置开机启动失败: {e:#}"));
                            }
                        }
                    }
                }
            }
        }
    }

    fn show_manage(&mut self, ui: &mut egui::Ui) {
        // B9: 多程序 Tab 栏
        self.show_program_tabs(ui);
        if self.manager.programs.len() > 1 {
            ui.add_space(2.0);
        }
        let Some(p) = self.shown_program().cloned() else {
            ui.label("未配置任何受管程序。请以 `universal-shell-egui <shell.json>` 启动，或在「模板库」导入。");
            return;
        };

        ui.heading(&p.name);
        ui.weak(&p.description);
        ui.separator();

        // 状态行
        let status = self.manager.status(&p);
        let mut chips = vec![
            format!("本地版本: {}", status.local_version),
            format!(
                "最新版本: {}",
                status.latest_version.as_deref().unwrap_or("未知")
            ),
            if status.installed { "已安装" } else { "未安装" }.to_string(),
        ];
        if self.manager.runner.is_running(&p.id) {
            chips.push("运行中".into());
        } else {
            chips.push("已停止".into());
        }
        ui.horizontal(|ui| {
            for c in chips {
                ui.label(c);
            }
        });
        ui.add_space(6.0);

        // 模板更新行（仅当实例带来源注册表时显示）
        if p.template_source.is_some() {
            ui.horizontal(|ui| {
                let checking = self.update_checks.get(&p.id).cloned();
                let btn = ui.add_enabled(
                    checking.is_none(),
                    egui::Button::new(if checking.is_some() { "检查中…" } else { "检查模板更新" }),
                );
                if btn.clicked() {
                    self.spawn_check_template_update(p.clone());
                    self.notice.insert(p.id.clone(), String::new());
                }
                if let Some((_, diff)) = self.pending_updates.get(&p.id).cloned() {
                    let mut detail = diff.changed_fields_detail.clone();
                    ui.label(diff.summary());
                    if ui.button("查看变化").clicked() {
                        self.notice.insert(
                            p.id.clone(),
                            if detail.is_empty() {
                                diff.summary()
                            } else {
                                detail.drain(..).collect::<Vec<_>>().join("；")
                            },
                        );
                    }
                    if ui.button("应用模板更新").clicked() {
                        if let Some((remote, _)) = self.pending_updates.get(&p.id).cloned() {
                            self.apply_pending_update(p.clone(), remote);
                        }
                    }
                }
            });
            ui.add_space(4.0);
        }

        // 配置驱动表单
        self.show_form(ui);
        ui.add_space(8.0);

        // 操作区
        ui.horizontal(|ui| {
            let download_btn = ui.add_enabled(
                !self.busy,
                egui::Button::new(if self.busy { "下载中…" } else { "下载 / 更新" }),
            );
            if download_btn.clicked() {
                self.spawn_install(p.clone());
                self.notice.insert(p.id.clone(), String::new());
            }

            let values = self.values.get(&p.id).cloned().unwrap_or_default();
            let values_for_start = values;
            if self.manager.runner.is_running(&p.id) {
                if ui.button("停止").clicked() {
                    if let Err(e) = self.manager.stop(&p.id) {
                        self.notice.insert(p.id.clone(), format!("{e:#}"));
                    } else {
                        self.notice.insert(p.id.clone(), String::new());
                    }
                }
            } else if ui.button("启动").clicked() {
                self.manager.save_field_values(&p, &values_for_start);
                match self.manager.start(&p, &values_for_start) {
                    Ok(()) => {
                        self.notice.insert(p.id.clone(), String::new());
                    }
                    Err(e) => {
                        self.notice.insert(p.id.clone(), format!("启动失败: {e:#}"));
                    }
                }
            }

            if ui.button("打开日志目录").clicked() {
                let log_dir = self.manager.data_dir.join("logs");
                let _ = std::process::Command::new(&open_cmd())
                    .arg(&log_dir)
                    .spawn();
            }
        });

        if let Some(n) = self.notice.get(&p.id) {
            if !n.is_empty() {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::from_rgb(200, 90, 90), n);
            }
        }
    }

    fn show_library(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("注册表 URL:");
            ui.add(
                egui::TextEdit::singleline(&mut self.registry_url)
                    .hint_text("https://…/templates/")
                    .desired_width(320.0),
            );
            let refresh = ui.add_enabled(
                !self.registry_wait,
                egui::Button::new(if self.registry_wait { "拉取中…" } else { "刷新" }),
            );
            if refresh.clicked() {
                self.spawn_load_manifest();
            }
        });

        let registry_hint: String =
            self.manager.template_registries.join(", ").trim().to_string();
        if !registry_hint.is_empty() {
            ui.weak(format!("配置的注册表: {registry_hint}"));
        }

        if self.manifest_offline {
            ui.colored_label(
                egui::Color32::from_rgb(200, 150, 50),
                "当前为离线(缓存)状态，显示的是上次成功拉取的清单",
            );
        }

        ui.separator();

        let Some(merged) = self.merged.clone() else {
            ui.label("尚未加载清单。点击「刷新」从注册表拉取(失败时回退本地缓存)。");
            return;
        };

        // 各源状态行
        for (base, off) in &merged.sources {
            if *off {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 150, 50),
                    format!("离线(缓存): {base}"),
                );
            } else {
                ui.weak(format!("源: {base}"));
            }
        }
        ui.add_space(4.0);

        // 搜索框
        ui.horizontal(|ui| {
            ui.label(format!(
                "共 {} 个模板（{} 个源，冲突 {}）",
                merged.template_count(),
                merged.sources.len(),
                merged.conflicts.len()
            ));
            ui.separator();
            ui.label("搜索:");
            ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(200.0));
        });
        ui.add_space(6.0);

        let keyword = self.search.trim().to_lowercase();
        let mut rows: Vec<(String, shared::TemplateIndex, String)> = Vec::new();
        for (id, (base, t)) in &merged.by_id {
            if !keyword.is_empty() {
                let hay = format!("{} {} {} {}", id, t.name, t.category, t.description)
                    .to_lowercase();
                if !hay.contains(&keyword) {
                    continue;
                }
            }
            rows.push((id.clone(), t.clone(), base.clone()));
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (id, t, base) in rows {
                ui.horizontal(|ui| {
                    let import_state = self.imports.get(&id).cloned();
                    match import_state {
                        None => {
                            if ui.button("导入").clicked() {
                                self.spawn_import_template(id.clone(), base.clone());
                            }
                        }
                        Some(state) => {
                            ui.add_enabled(false, egui::Button::new(state));
                        }
                    }
                    ui.strong(&t.name);
                    ui.label(format!("[{}]", t.category));
                    ui.weak(&t.repo);
                    // C5: 冲突标记
                    let conflicts = merged.id_conflicts(&id);
                    if conflicts.len() > 1 {
                        ui.colored_label(
                            egui::Color32::from_rgb(200, 150, 50),
                            "⚠ 多源",
                        );
                        if ui.small_button("?来源").clicked() {
                            self.notice.insert(
                                "__registry__".into(),
                                format!("「{id}」存在于: {}", conflicts.join(" ; ")),
                            );
                        }
                    }
                });
                ui.label(&t.description);
                ui.weak(format!("id: {id}"));
                ui.separator();
            }
        });
    }
}

impl eframe::App for ShellApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_msgs();
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
        ui.horizontal(|ui| {
            ui.heading("Universal Shell");
            ui.separator();
            if ui.selectable_label(matches!(self.view, View::Manage), "程序管理").clicked() {
                self.view = View::Manage;
            }
            if ui.selectable_label(matches!(self.view, View::Library), "模板库").clicked() {
                self.view = View::Library;
            }
            ui.separator();
            if ui.button("刷新状态").clicked() {
                self.refresh_status();
            }
            if ui.button("停止所有").clicked() {
                self.manager.stop_all();
            }
            ui.weak(format!("数据目录: {}", self.manager.data_dir.display()));
        });
        ui.separator();

        match self.view {
            View::Manage => self.show_manage(ui),
            View::Library => self.show_library(ui),
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