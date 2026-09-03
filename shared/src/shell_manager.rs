//! 壳级协调层：数据目录、安装/更新流程、对 UI 暴露的高层接口。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use log::info;
use rust_i18n::t;

use crate::autostart::AutoStart;
use crate::config::{ExtractMode, FieldKind, Program, ShellConfig};
use crate::extract;
use crate::github::{GitHub, LatestRelease};
use crate::runner::Runner;

/// 内置模板注册表（bash 配置为空时的默认源）。
/// 指向本仓库 `registry/` 的 GitHub raw，配对的 Ed25519 公钥见
/// [`DEFAULT_REGISTRY_PUBKEY`]。
pub const DEFAULT_REGISTRY: &str =
    "https://raw.githubusercontent.com/Jonnyan404/universal-shell/main/registry/";
/// 配套内置注册表的 demo 公钥（sign_registry 固定测试种子对应公钥）。
pub const DEFAULT_REGISTRY_PUBKEY: &str =
    "4cb5abf6ad79fbf5abbccafcc269d85cd2651ed4b885b5869f241aedf0a5ba29";

/// 运行状态快照，供 UI 显示
#[derive(Debug, Clone)]
pub struct ProgramStatus {
    pub installed: bool,
    pub running: bool,
    pub local_version: String,
    pub latest_version: Option<String>,
    pub latest_published: String,
}

/// 实例与远端模板的差异(供「应用模板更新」前展示)
#[derive(Debug, Clone, Default)]
pub struct TemplateDiff {
    /// 变更是何类内容引起（可空，表示模板定义本身无变化）
    pub changed_repo: bool,
    pub changed_assets: bool,
    pub changed_args: bool,
    pub changed_fields: bool,
    pub changed_fields_detail: Vec<String>,
    pub changed_arch_map: bool,
    pub changed_os_map: bool,
    pub changed_working_dir: bool,
    pub changed_version_pin: bool,
}

impl TemplateDiff {
    pub fn is_empty(&self) -> bool {
        !self.changed_repo
            && !self.changed_assets
            && !self.changed_args
            && !self.changed_fields
            && !self.changed_arch_map
            && !self.changed_os_map
            && !self.changed_working_dir
            && !self.changed_version_pin
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.changed_repo {
            parts.push("repo".to_string());
        }
        if self.changed_assets {
            parts.push(t!("tmpl.assets").to_string());
        }
        if self.changed_args {
            parts.push(t!("tmpl.args").to_string());
        }
        if self.changed_fields {
            parts.push(t!("tmpl.fields").to_string());
        }
        if self.changed_arch_map || self.changed_os_map {
            parts.push(t!("tmpl.arch_os").to_string());
        }
        if self.changed_working_dir {
            parts.push(t!("tmpl.workdir").to_string());
        }
        if self.changed_version_pin {
            parts.push(t!("tmpl.version_pin").to_string());
        }
        if parts.is_empty() {
            t!("tmpl.none").to_string()
        } else {
            t!("tmpl.changed", parts = parts.join("、")).to_string()
        }
    }
}

pub struct ShellManager {
    /// 数据目录：存放下载的二进制、日志、运行时配置
    pub data_dir: PathBuf,
    pub programs: Vec<Program>,
    /// 远程模板注册表基地址列表
    pub template_registries: Vec<String>,
    /// 各注册表 base -> Ed25519 公钥(hex)，签名校验
    pub registry_pubkeys: BTreeMap<String, String>,
    pub github: GitHub,
    pub runner: Runner,
    pub autostart: AutoStart,
    /// 网络代理/加速设置（增量应用到 github 与注册表请求）
    pub proxy: crate::config::ProxySettings,
    /// 界面语言：`auto`（跟随系统）/ `zh-CN` / `en`
    pub locale: String,
}

impl ShellManager {
    pub fn new(data_dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&data_dir)
            .with_context(|| t!("err.datadir", path = data_dir.display().to_string()))?;
        Ok(Self {
            data_dir,
            programs: vec![],
            // 默认内置官方 GitHub 源(附 demo 公钥验签)，保证全新数据目录也能看到官方模板；
            // 若后续加载了 shell.json 且用户显式配置了其它源，由 load_config 覆盖之。
            template_registries: vec![DEFAULT_REGISTRY.to_string()],
            registry_pubkeys: [(DEFAULT_REGISTRY.to_string(), DEFAULT_REGISTRY_PUBKEY.to_string())]
                .into_iter()
                .collect(),
            github: GitHub::default(),
            runner: Runner::new(),
            autostart: AutoStart::new(),
            proxy: crate::config::ProxySettings::default(),
            locale: "auto".to_string(),
        })
    }

    /// 加载配置文件(JSON)。支持 `--config` 路径，默认从数据目录读 shell.json
    pub fn load_config(&mut self, path: &Path) -> anyhow::Result<()> {
        info!("{}", t!("log.config.load", path = path.display()));
        let cfg = ShellConfig::load(&path.to_path_buf())?;
        self.programs = cfg.programs;
        // 未显式配置注册表时使用内置 GitHub 源（附 demo 公钥签名校验）
        self.template_registries = if cfg.template_registries.is_empty() {
            vec![DEFAULT_REGISTRY.to_string()]
        } else {
            cfg.template_registries
        };
        let mut pubkeys = cfg.registry_pubkeys;
        if !pubkeys
            .keys()
            .any(|b| self.template_registries.iter().any(|r| r == b))
        {
            pubkeys.insert(DEFAULT_REGISTRY.to_string(), DEFAULT_REGISTRY_PUBKEY.to_string());
        }
        self.registry_pubkeys = pubkeys;
        // 应用网络代理设置到 GitHub 客户端
        self.github.apply_network(
            &cfg.proxy.accelerate_prefix,
            &cfg.proxy.http_proxy,
        );
        self.proxy = cfg.proxy;
        self.locale = if cfg.locale.is_empty() { "auto".to_string() } else { cfg.locale };
        info!("{}", t!("log.config.loaded", count = self.programs.len()));
        Ok(())
    }

    /// 将当前程序列表 + 注册表/代理配置写回到 shell.json
    pub fn save_config(&self, path: &Path) -> anyhow::Result<()> {
        let cfg = ShellConfig {
            programs: self.programs.clone(),
            template_registries: self.template_registries.clone(),
            registry_pubkeys: self.registry_pubkeys.clone(),
            proxy: self.proxy.clone(),
            locale: self.locale.clone(),
        };
        let json = serde_json::to_string_pretty(&cfg).context(t!("err.serialize_config"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, json)
            .with_context(|| t!("err.write_config", path = path.display().to_string()))?;
        info!("{}", t!("log.config.saved", path = path.display()));
        Ok(())
    }

    pub fn default_config_path(&self) -> PathBuf {
        self.data_dir.join("shell.json")
    }

    /// 程序专属数据目录(下载的二进制、版本、字段值、整包解压都归于此，与其它应用隔离)。
    /// 也是该程序以默认工作目录(working_dir=.)运行时的工作目录，避免各应用把配置文件
    /// 都写到数据根目录、同名文件互相覆盖。
    pub fn app_dir(&self, p: &Program) -> PathBuf {
        self.data_dir.join(&p.id)
    }

    /// 单独可执行文件在程序目录 bin/ 子目录(与 whole 模式的解包目录 pkg/ 并存不冲突)。
    pub fn bin_path(&self, p: &Program) -> PathBuf {
        let name = if std::env::consts::OS == "windows" {
            format!("{}.exe", p.binary)
        } else {
            p.binary.clone()
        };
        self.app_dir(p).join("bin").join(&name)
    }

    fn log_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    /// 壳自身操作日志文件（启动/停止/自启动/导入/设置等统一落盘）
    pub fn op_log_path(&self) -> PathBuf {
        self.data_dir.join("shell.log")
    }

    /// 追加一条壳操作日志。带本地时间戳；落盘失败仅记录，不影响功能。
    pub fn log_op(&self, msg: &str) {
        let stamp = || {
            use time::format_description::well_known::Rfc3339;
            let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
            now.format(&Rfc3339).unwrap_or_else(|_| "?".into())
        };
        let line = format!("[{}] {}\n", stamp(), msg);
        let path = self.op_log_path();
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = f.write_all(line.as_bytes());
        } else {
            log::warn!("{}", t!("log.op_write_fail", path = path.display()));
        }
    }

    /// 清空壳操作日志
    pub fn clear_op_log(&self) {
        let _ = std::fs::write(self.op_log_path(), b"");
    }

    /// 版本检查缓存文件路径（repo -> (最新版本, 检查时间戳)）
    pub fn version_check_path(&self) -> PathBuf {
        self.data_dir.join("cache").join("version-check.json")
    }

    /// 读取版本检查缓存（repo -> (latest_version, checked_at)）
    pub fn load_version_check(&self) -> BTreeMap<String, (String, u64)> {
        std::fs::read_to_string(self.version_check_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// 保存版本检查缓存（供“检查更新”后落盘，避免重复联网）
    pub fn save_version_check(&self, map: &BTreeMap<String, (String, u64)>) {
        let path = self.version_check_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(t) = serde_json::to_string_pretty(map) {
            let _ = std::fs::write(&path, t);
        }
    }

    /// 清空某程序的日志文件（显式“停止”时调用；重启不清空）
    pub fn clear_log(&self, id: &str) {
        let _ = std::fs::write(self.log_dir().join(format!("{id}.log")), b"");
    }

    // ---------- 安装 / 更新 ----------

    /// 安装或更新程序到最新版。返回 (本地版本号, 最新版本号)
    ///
    /// 从 GitHub 拉最新版本，下载匹配当前 OS/架构的资产，解压出成员落盘为
    /// bin_path(与壳不同名，天然防覆盖)。已装同类文件时保留旧二进制为 .old 便于回滚。
    pub fn install_or_update(
        &self,
        program: &Program,
        on_progress: &dyn Fn(&crate::progress::DownloadProgress),
    ) -> anyhow::Result<(String, String)> {
        // 1. 查最新版本
        let release: LatestRelease = self.github.latest(&program.repo)?;
        let version = release.tag_name.trim_start_matches('v').to_string();
        info!("{}", t!("log.version_latest", id = &program.id, version = &version));

        // 1b. 已安装且本地版本不低于最新版时，跳过重复下载
        if !crate::version::is_newer(&version, &self.local_version(program))
            && self.bin_path(program).exists()
        {
            info!("{}", t!("log.version_skip", id = &program.id));
            return Ok((version.clone(), version));
        }

        // 2. 候选匹配资产名/URL/digest
        let arch = std::env::consts::ARCH.to_string();
        let (rule, asset_name, url, api_digest) = self.github.resolve_download(program, &arch, &version)?;

        // 3. 下载到临时文件
        let dl_path = self.data_dir.join(format!(".dl-{}", &asset_name));
        info!("{}", t!("log.downloading", url = url, path = dl_path.display()));
        use crate::progress::{DownloadProgress, DownloadStage};
        on_progress(&DownloadProgress::stage(DownloadStage::Downloading));
        self.github.download_to_with_progress(&url, &dl_path, &|received, total| {
            on_progress(&DownloadProgress::downloading(received, total));
        })?;
        info!("{}", t!("log.download_done", bytes = std::fs::metadata(&dl_path).map(|m| m.len()).unwrap_or(0)));

        // 3b. sha256 校验：优先用 GitHub API 下发的 digest；模板显式 check_sha256 钉住时以它为准
        on_progress(&DownloadProgress::stage(DownloadStage::Verifying));
        let expect = match (&program.check_sha256, &api_digest) {
            (Some(pin), _) => Some(pin.clone()),
            (None, Some(d)) => Some(d.clone()),
            (None, None) => None,
        };
        if let Some(expect) = expect {
            if let Err(e) = crate::checksum::verify_download(&dl_path, &expect) {
                let _ = std::fs::remove_file(&dl_path);
                return Err(e.context(t!(
                    "err.asset_validate",
                    path = dl_path.display().to_string()
                )));
            }
            info!("{}", t!("log.checksum_ok", id = &program.id));
        }

        // 4. 若为 whole 模式则整包解到 <id> 目录，再令 bin 指向其中 member
        on_progress(&DownloadProgress::stage(DownloadStage::Extracting));
        let run_target = self.bin_path(program);
        let package_dir = self.app_dir(program).join("pkg");
        if let Some(parent) = run_target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match rule.mode {
            ExtractMode::Single => {
                // 直接抽单成员落盘为 run_target
                if run_target.exists() {
                    let old = run_target.with_extension("old");
                    let _ = std::fs::remove_file(&old);
                    let _ = std::fs::rename(&run_target, &old);
                }
                let member = rule.member.as_deref();
                extract::extract_file(&dl_path, &rule.format, member, &run_target)
                    .with_context(|| t!("err.extract", path = dl_path.display().to_string()))?;
            }
            ExtractMode::Whole => {
                // 清空旧解包目录(避免残留旧版多余文件)
                if package_dir.exists() {
                    let _ = std::fs::remove_dir_all(&package_dir);
                }
                extract::extract_whole(&dl_path, &rule.format, &package_dir)
                    .with_context(|| t!("err.extract", path = dl_path.display().to_string()))?;
                // 在壳数据目录生成指向包内成员的可执行入口(与壳不同名，防覆盖壳自身)
                let member_tpl = rule.member.as_deref().unwrap_or(&program.binary);
                let inner = program.render_template(member_tpl, &version, &rule, &arch);
                let inner_real = resolve_member(&package_dir, &inner);
                let _ = std::fs::remove_file(&run_target);
                if let Some(parent) = run_target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if std::fs::hard_link(&inner_real, &run_target).is_err() {
                    std::fs::copy(&inner_real, &run_target)
                        .with_context(|| t!("err.copy_exe"))?;
                }
                set_exec(run_target.as_path());
            }
            ExtractMode::Raw => {
                if run_target.exists() {
                    let old = run_target.with_extension("old");
                    let _ = std::fs::remove_file(&old);
                    let _ = std::fs::rename(&run_target, &old);
                }
                std::fs::copy(&dl_path, &run_target)
                    .with_context(|| t!("err.copy_bin", path = dl_path.display().to_string()))?;
                set_exec(run_target.as_path());
            }
        }

        // 5. 清理临时文件
        let _ = std::fs::remove_file(&dl_path);

        // 6. 记录本地版本
        self.save_local_version(program, &version);
        Ok((version.clone(), version))
    }

    /// 独立(data_dir + Program)的安装入口，供线程/CLI 使用。返回最新版本号。
    pub fn install_standalone(data_dir: &PathBuf, program: &Program) -> anyhow::Result<String> {
        Self::install_standalone_with_progress(data_dir, program, &|_| {})
    }

    /// 独立安装 + 进度回调（WebView/CLI 展示进度用）。
    pub fn install_standalone_with_progress(
        data_dir: &PathBuf,
        program: &Program,
        on_progress: &dyn Fn(&crate::progress::DownloadProgress),
    ) -> anyhow::Result<String> {
        let mut mgr = ShellManager::new(data_dir.clone())?;
        // 若存在 shell.json，则读取并应用其网络代理设置
        let cfg_path = data_dir.join("shell.json");
        if cfg_path.exists() {
            if let Ok(cfg) = crate::config::ShellConfig::load(&cfg_path) {
                mgr.github
                    .apply_network(&cfg.proxy.accelerate_prefix, &cfg.proxy.http_proxy);
            }
        }
        let (version, _) = mgr.install_or_update(program, on_progress)?;
        Ok(version)
    }

    pub fn save_local_version(&self, program: &Program, version: &str) {
        let dir = self.app_dir(program);
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let vs_file = dir.join("version");
        if let Ok(mut f) = std::fs::File::create(&vs_file) {
            use std::io::Write;
            let _ = f.write_all(version.as_bytes());
        }
    }

    pub fn local_version(&self, program: &Program) -> String {
        let vs_file = self.app_dir(program).join("version");
        std::fs::read_to_string(vs_file)
            .map(|s| s.trim().trim_start_matches('v').to_string())
            .unwrap_or_else(|_| "-".to_string())
    }

    /// 查询某程序状态(本地是否已装、是否运行、版本对比)
    pub fn status(&mut self, program: &Program) -> ProgramStatus {
        let mut st = self.status_local(program);
        if let Ok(latest) = self.github.latest(&program.repo) {
            st.latest_version = Some(latest.tag_name.trim_start_matches('v').to_string());
            st.latest_published = latest.published_at.clone();
        }
        st
    }

    /// 该程序是否在运行：壳持有子进程句柄，或系统上仍有该程序的进程
    /// （壳上次退出后残留、仍在后台运行）。避免只查句柄而漏判孤儿进程。
    pub fn is_program_running(&mut self, program: &Program) -> bool {
        self.runner.is_running(&program.id) || self.runner.is_process_alive(&self.bin_path(program))
    }

    /// 仅本地状态（不发网络请求）：安装、运行、本地版本。
    /// 供 UI 在不持锁情况下先渲染，再异步补全最新版本。
    pub fn status_local(&mut self, program: &Program) -> ProgramStatus {
        let installed = self.bin_path(program).exists();
        let running = self.is_program_running(program);
        let local_version = self.local_version(program);
        ProgramStatus {
            installed,
            running,
            local_version,
            latest_version: None,
            latest_published: String::new(),
        }
    }

    // ---------- 启动 / 停止 ----------

    /// 用字段值渲染 args 并启动子进程
    pub fn start(
        &mut self,
        program: &Program,
        field_values: &BTreeMap<String, String>,
    ) -> anyhow::Result<()> {
        let bin = self.bin_path(program);
        if !bin.exists() {
            return Err(anyhow::anyhow!(t!("err.not_installed", name = &program.name)));
        }
        // 若壳持有句柄 → 已在运行，直接报错
        // 若壳无句柄但系统仍有该程序进程（壳上次退出后残留、仍在后台运行）→
        // 不杀不重启，提示已在运行，避免误杀用户保留的常驻进程。
        let held = self.runner.is_running(&program.id);
        let orphan = !held && self.runner.is_process_alive(&bin);
        if held {
            return Err(anyhow::anyhow!(t!("err.already_running", name = &program.name)));
        }
        if orphan {
            return Err(anyhow::anyhow!(t!(
                "err.running_bg",
                name = &program.name
            )));
        }
        let missing: Vec<&str> = program
            .fields
            .iter()
            .filter(|f| f.required)
            .filter(|f| {
                let v = field_values.get(&f.key);
                !v.map(|s| !s.trim().is_empty()).unwrap_or(false)
            })
            .map(|f| f.label())
            .collect();
        if !missing.is_empty() {
            return Err(anyhow::anyhow!(t!(
                "err.missing_required",
                name = &program.name,
                missing = missing.join("、")
            )));
        }
        let args = program.render_args(field_values);
        let wd: PathBuf = if program.working_dir == "." {
            let dir = self.app_dir(program);
            let _ = std::fs::create_dir_all(&dir);
            dir
        } else {
            PathBuf::from(&program.working_dir)
        };
        self.runner
            .start_async(&program.id, &bin, &args, &wd, &self.log_dir())
    }

    pub fn stop(&mut self, id: &str) -> anyhow::Result<()> {
        // 优先停掉壳持有的子进程句柄；若壳无句柄（如上次退出后该程序仍在后台
        // 运行），则按可执行文件路径杀掉残留进程，确保能真正停掉、可再重启。
        match self.runner.stop(id) {
            Ok(()) => return Ok(()),
            Err(e1) => {
                let bin = self
                    .programs
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| self.bin_path(p));
                if let Some(bin) = bin {
                    if self.runner.kill_orphan_by_path(&bin) {
                        log::info!("{}", t!("log.stale_killed", id = id));
                        return Ok(());
                    }
                }
                Err(e1)
            }
        }
    }

    pub fn stop_all(&mut self) {
        self.runner.stop_all();
    }

    // ---------- 字段值持久化 ----------

    /// 读取程序上次保存的字段值(合并运行时默认)
    pub fn load_field_values(&self, program: &Program) -> BTreeMap<String, String> {
        let mut map = program.runtime_defaults();
        let fv_file = self.app_dir(program).join("values.json");
        if let Ok(raw) = std::fs::read_to_string(&fv_file) {
            if let Ok(extra) = serde_json::from_str::<BTreeMap<String, String>>(&raw) {
                map.extend(extra);
            }
        }
        map
    }

    /// 保存字段值
    pub fn save_field_values(&self, program: &Program, values: &BTreeMap<String, String>) {
        let dir = self.app_dir(program);
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let fv_file = dir.join("values.json");
        if let Ok(raw) = serde_json::to_string_pretty(values) {
            let _ = std::fs::write(&fv_file, raw);
        }
    }

    /// 读取程序「自启动」字段的当前值（方案 B：由壳启动时拉起，而非系统登录项）
    pub fn program_autostart(&self, program: &Program) -> bool {
        let key = program
            .fields
            .iter()
            .find(|f| matches!(f.kind, FieldKind::AutoStart { .. }))
            .map(|f| f.key.clone());
        match key {
            Some(k) => self
                .load_field_values(program)
                .get(&k)
                .map(|v| v == "true")
                .unwrap_or(false),
            None => false,
        }
    }

    /// Autostart 字段:若字段值是 true 则注册登录项，否则关闭
    /// 方案 B（壳管理）：程序的「自启动」不再注册系统登录项，而是作为
    /// 「壳启动时自动拉起」的清单。启用状态由字段值持久化（见 set_autostart）。
    /// 此函数保留为空操作，避免误注册被管程序到系统登录项。
    pub fn apply_key_autostart(&mut self, _program: &Program, _values: &BTreeMap<String, String>) -> anyhow::Result<()> {
        Ok(())
    }

    /// 壳启动后自动拉起所有开启「自启动」的程序（方案 B：壳管理）。    /// 逐个用保存的字段值渲染参数并启动，失败仅记日志不影响后续。
    pub fn start_autostart_programs(&mut self) {
        let programs = self.programs.clone();
        for p in &programs {
            let values = self.load_field_values(p);
            let enabled = p
                .fields
                .iter()
                .find(|f| matches!(f.kind, FieldKind::AutoStart { .. }))
                .map(|f| values.get(&f.key).map(|v| v == "true").unwrap_or(false))
                .unwrap_or(false);
            if !enabled {
                continue;
            }
            let bin = self.bin_path(p);
            let held = self.runner.is_running(&p.id);
            let orphan = !held && self.runner.is_process_alive(&bin);
            if held || orphan {
                continue; // 已在运行/残留进程在跑，不重复拉起
            }
            match self.start(p, &values) {
                Ok(()) => {
                    log::info!("{}", t!("log.autostart.launched", name = &p.name));
                    self.log_op(&t!("log.autostart.launch", name = &p.name));
                }
                Err(e) => {
                    log::warn!("{}", t!("log.autostart.launch_failed", name = &p.name, err = e));
                    self.log_op(&t!("log.autostart.fail", name = &p.name, err = e.to_string()));
                }
            }
        }
    }

    // ---------- 模板更新 (C4) ----------

    /// 对比实例当前定义与远端模板定义，产出差异摘要。比较结构层面：repo/资产/参数/字段/映射。
    pub fn template_diff(&self, current: &Program, remote: &Program) -> TemplateDiff {
        let mut d = TemplateDiff::default();
        let eqkv = |a: &serde_json::Value, b: &serde_json::Value| a == b;
        d.changed_repo = current.repo != remote.repo;
        d.changed_assets = serde_json::to_value(&current.assets).unwrap_or_default()
            != serde_json::to_value(&remote.assets).unwrap_or_default();
        d.changed_args = current.args != remote.args;
        d.changed_arch_map = current.arch_map != remote.arch_map;
        d.changed_os_map = current.os_map != remote.os_map;
        d.changed_working_dir = current.working_dir != remote.working_dir;
        d.changed_version_pin = current.check_sha256 != remote.check_sha256;
        // 字段变化：逐条比对 key + kind
        let cur: BTreeMap<&str, &crate::config::Field> =
            current.fields.iter().map(|f| (f.key.as_str(), f)).collect();
        let rem: BTreeMap<&str, &crate::config::Field> =
            remote.fields.iter().map(|f| (f.key.as_str(), f)).collect();
        if !eqkv(&serde_json::to_value(&current.fields).unwrap_or_default(), &serde_json::to_value(&remote.fields).unwrap_or_default())
        {
            d.changed_fields = true;
        }
        for key in rem.keys() {
            match cur.get(key) {
                None => d.changed_fields_detail.push(t!(
                    "diff.add_field",
                    key = key,
                    label = rem[key].label(),
                    def = rem[key].default_raw()
                ).to_string()),
                Some(cf) if !eqkv(&serde_json::to_value(&cf.kind).unwrap_or_default(), &serde_json::to_value(&rem[key].kind).unwrap_or_default()) => {
                    d.changed_fields_detail.push(t!("diff.field_changed", key = key, label = rem[key].label()).to_string());
                }
                _ => {}
            }
        }
        for key in cur.keys() {
            if !rem.contains_key(key) {
                d.changed_fields_detail.push(t!("diff.remove_field", key = key, label = cur[key].label()).to_string());
            }
        }
        d
    }

    /// 把远端模板应用到实例：用远端定义替换当前实例的结构，但保留用户已填的字段值。
    /// 返回合并后的字段值(含新增字段默认值)。
    pub fn apply_template_update(
        current: &mut Program,
        remote: &Program,
        current_values: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        // 结构字段整体替换
        current.repo = remote.repo.clone();
        current.binary = remote.binary.clone();
        current.assets = remote.assets.clone();
        current.arch_map = remote.arch_map.clone();
        current.os_map = remote.os_map.clone();
        current.fields = remote.fields.clone();
        current.args = remote.args.clone();
        current.working_dir = remote.working_dir.clone();
        current.check_sha256 = remote.check_sha256.clone();
        current.description = remote.description.clone();
        current.category = remote.category.clone();
        // 名称一般不变，但可同步
        current.name = remote.name.clone();

        // 字段值迁移：保留仍存在的 key，新增字段补默认值
        let mut merged = current_values.clone();
        let keys: Vec<String> = current_values.keys().cloned().collect();
        let live: Vec<&str> = remote.fields.iter().map(|f| f.key.as_str()).collect();
        for k in &keys {
            if !live.contains(&k.as_str()) {
                merged.remove(k);
            }
        }
        for f in &remote.fields {
            merged.entry(f.key.clone()).or_insert_with(|| f.default_raw());
        }
        merged
    }

    // ---------- 本地实例管理 ----------

    /// 用新定义替换实例（完整编辑 name/description/repo/args/fields 等），
    /// 保留 id 与 hidden 状态，并迁移/补齐字段值 + 写回配置。
    pub fn update_program(
        &mut self,
        id: &str,
        remote: &Program,
        path: &Path,
    ) -> anyhow::Result<()> {
        let Some(idx) = self.programs.iter().position(|p| p.id == id) else {
            anyhow::bail!(t!("err.program_not_found", id = &id));
        };
        let old_values = self.load_field_values(&self.programs[idx]);
        let hidden = self.programs[idx].hidden;
        let merged = Self::apply_template_update(&mut self.programs[idx], remote, &old_values);
        self.programs[idx].hidden = hidden;
        // id 跟随实例(用户不改 id)，避免改动脏掉关联文件
        self.programs[idx].id = id.to_string();
        // apply_template_update 已按 remote.fields 迁移/补齐字段值
        self.save_field_values(&self.programs[idx], &merged);
        self.save_config(path)
    }

    /// 删除实例：从配置移除程序，并清理其二进制/版本/字段值文件。
    pub fn delete_program(&mut self, id: &str, path: &Path) -> anyhow::Result<()> {
        let Some(idx) = self.programs.iter().position(|p| p.id == id) else {
            anyhow::bail!(t!("err.program_not_found", id = &id));
        };
        let p = self.programs.remove(idx);
        if let Err(e) = self.runner.stop(&id) {
            log::warn!("{}", t!("log.stop_failed", id = id, err = format!("{e:#}")));
        }
        // 清理该程序专属数据目录（二进制/版本/字段值/整包解压都在一处）
        let removed = std::fs::remove_dir_all(self.app_dir(&p));
        if removed.is_err() {
            let _ = std::fs::remove_file(&self.bin_path(&p));
            let _ = std::fs::remove_file(self.app_dir(&p).join("version"));
            let _ = std::fs::remove_file(self.app_dir(&p).join("values.json"));
        }
        self.save_config(path)
    }

    /// 设置实例的隐藏状态；写出配置。隐藏后侧栏/主管理不展示，
    /// 批量管理始终可见，故不会出现「全部程序失联」的情况。
    pub fn set_hidden(&mut self, id: &str, hidden: bool, path: &Path) -> anyhow::Result<()> {
        let Some(p) = self.programs.iter_mut().find(|p| p.id == id) else {
            anyhow::bail!(t!("err.program_not_found", id = &id));
        };
        p.hidden = hidden;
        self.save_config(path)
    }

    /// 读取某程序合并日志（stdout/stderr 同文件；stderr 行以 \x1F 开头）。返回 (merged, 空)。
    pub fn read_logs(&self, id: &str, tail_bytes: usize) -> (String, String) {
        fn read_tail(p: &Path, n: usize) -> String {
            let Ok(data) = std::fs::read(p) else {
                return String::new();
            };
            // 从尾部截取最近 n 字节，尽量从换行处开始
            let start = data.len().saturating_sub(n);
            let mut s = data[start..].to_vec();
            if start > 0 {
                if let Some(pos) = s.iter().position(|&b| b == b'\n') {
                    s.drain(..=pos);
                }
            }
            String::from_utf8_lossy(&s).to_string()
        }
        let out = read_tail(&self.log_dir().join(format!("{id}.log")), tail_bytes);
        (out, String::new())
    }
}

impl Default for ProgramStatus {
    fn default() -> Self {
        Self {
            installed: false,
            running: false,
            local_version: "-".into(),
            latest_version: None,
            latest_published: String::new(),
        }
    }
}

/// 在解包目录内以 member(可能含前缀) 匹配真实文件路径；找不到回退直接拼接
fn resolve_member(package_dir: &Path, member: &str) -> PathBuf {
    let joined = package_dir.join(strip_leading(member));
    if joined.exists() {
        return joined;
    }
    let leaf = member.rsplit('/').next().unwrap_or(member).to_string();
    if let Some(found) = crate::extract::list_entries(&package_dir.to_path_buf())
        .into_iter()
        .find(|f| f.file_name().map(|n| n == leaf.as_str()).unwrap_or(false))
    {
        return found;
    }
    joined
}

fn strip_leading(p: &str) -> &str {
    p.strip_prefix("./").unwrap_or(p)
}

fn set_exec(p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(windows)]
    {
        let _ = p;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Field, FieldKind, Program};

    fn prog_copy(p: &Program) -> Program {
        serde_json::from_str(&serde_json::to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn diff_detects_changes_and_apply_preserves_user_values() {
        let mut current: Program = serde_json::from_str(r#"{
            "id":"dufs","name":"dufs","repo":"sigoden/dufs","binary":"dufs",
            "assets":{"darwin":{"candidates":["dufs-v{version}-x86_64-apple-darwin.tar.gz"],"format":"tar.gz","mode":"single"}},
            "fields":[{"key":"port","kind":"string","label":"端口","default":"5000"}],
            "args":["-p","{port}"]
        }"#).unwrap();

        // 远端模板：修改 args、新增字段、repo 不变
        let mut remote = prog_copy(&current);
        remote.args = vec!["--bind".to_string(), "127.0.0.1".to_string(), "-p".to_string(), "{port}".into()];
        remote.fields.push(Field { key: "bind".into(), kind: FieldKind::String { label: "绑定".into(), default: "127.0.0.1".into(), placeholder: String::new() }, required: false });

        let mgr = ShellManager::new(std::env::temp_dir().join("cc-c4-unknown")).unwrap();
        let diff = mgr.template_diff(&current, &remote);
        assert!(diff.changed_args);
        assert!(diff.changed_fields);
        assert!(!diff.is_empty());
        assert!(diff.changed_fields_detail.iter().any(|d| d.contains("bind")));

        // 用户填过 port,apply 后应保留 port 值并补 bind 默认
        let mut values = BTreeMap::new();
        values.insert("port".into(), "8888".into());
        let merged = ShellManager::apply_template_update(&mut current, &remote, &values);
        assert_eq!(merged.get("port").unwrap(), "8888");
        assert_eq!(merged.get("bind").unwrap(), "127.0.0.1");
        assert_eq!(current.args, remote.args);
        assert_eq!(current.fields.len(), 2);
    }

    #[test]
    fn diff_identical_is_empty() {
        let p: Program = serde_json::from_str(r#"{"id":"x","name":"X","repo":"a/b","binary":"x1","fields":[],"args":[]}"#).unwrap();
        // 需有匹配 os 的 assets(否则 identity map) —— 直接复制相同 JSON 确保完全一致
        let q = prog_copy(&p);
        let mgr = ShellManager::new(std::env::temp_dir().join("cc-c4-noop")).unwrap();
        assert!(mgr.template_diff(&p, &q).is_empty());
    }

    #[test]
    fn empty_registries_falls_back_to_builtin() {
        let dir = std::env::temp_dir().join("cc-builtin-registry");
        let mut mgr = ShellManager::new(dir.clone()).unwrap();
        let empty = dir.join("empty.json");
        std::fs::write(&empty, r#"{"programs":[]}"#).unwrap();
        mgr.load_config(&empty).unwrap();
        assert_eq!(mgr.template_registries, vec![DEFAULT_REGISTRY]);
        assert_eq!(
            mgr.registry_pubkeys.get(DEFAULT_REGISTRY).map(|s| s.as_str()),
            Some(DEFAULT_REGISTRY_PUBKEY)
        );
    }
}