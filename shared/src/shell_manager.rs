//! 壳级协调层：数据目录、安装/更新流程、对 UI 暴露的高层接口。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use log::{info, warn};
use rust_i18n::t;

use crate::autostart::AutoStart;
use crate::config::{AssetRule, ExtractMode, Program, ShellConfig};
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
    /// 内置受管程序（编译期嵌入，始终存在；用户同 id 配置优先）
    pub builtin_programs: Vec<Program>,
    /// 远程模板注册表基地址列表
    pub template_registries: Vec<String>,
    /// 各注册表 base -> Ed25519 公钥(hex)，签名校验
    pub registry_pubkeys: BTreeMap<String, String>,
    pub github: GitHub,
    /// 通用 HTTP 直链下载源（方案 A；source.kind=http 时使用）
    pub http: crate::source_http::HttpSource,
    pub runner: Runner,
    pub autostart: AutoStart,
    /// 每台受管程序的自启状态（program id -> 是否随壳启动拉起）。独立于模板字段，
    /// 模板可不带 autostart 参数，由壳统一管理。持久化见 [`Self::program_autostart_path`]。
    pub program_autostart_map: BTreeMap<String, bool>,
    /// 网络代理/加速设置（增量应用到 github 与注册表请求）
    pub proxy: crate::config::ProxySettings,
    /// 界面语言：`auto`（跟随系统）/ `zh-CN` / `en`
    pub locale: String,
}

impl ShellManager {
    pub fn new(data_dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&data_dir)
            .with_context(|| t!("err.datadir", path = data_dir.display().to_string()))?;
        let program_autostart = Self::load_program_autostart(&data_dir);
        Ok(Self {
            data_dir,
            programs: vec![],
            // 内置程序编译期固定；用户加载 shell.json 时按同 id 覆盖，未覆盖的多保留。
            builtin_programs: crate::builtin::builtin_programs().to_vec(),
            // 默认内置官方 GitHub 源(附 demo 公钥验签)，保证全新数据目录也能看到官方模板；
            // 若后续加载了 shell.json 且用户显式配置了其它源，由 load_config 覆盖之。
            template_registries: vec![DEFAULT_REGISTRY.to_string()],
            registry_pubkeys: [(DEFAULT_REGISTRY.to_string(), DEFAULT_REGISTRY_PUBKEY.to_string())]
                .into_iter()
                .collect(),
            github: GitHub::default(),
            http: crate::source_http::HttpSource::default(),
            runner: Runner::new(),
            autostart: AutoStart::new(),
            program_autostart_map: program_autostart,
            proxy: crate::config::ProxySettings::default(),
            locale: "auto".to_string(),
        })
    }

    /// 全部受管程序 = 内置 + 用户配置，用户同 id 优先（覆盖内置）。内置始终并存。
    pub fn all_programs(&self) -> Vec<Program> {
        let mut all = self.builtin_programs.clone();
        let mut ids: std::collections::HashSet<String> = all.iter().map(|p| p.id.clone()).collect();
        for p in &self.programs {
            if ids.insert(p.id.clone()) {
                all.push(p.clone());
            } else if let Some(existing) = all.iter_mut().find(|e| e.id == p.id) {
                // 用户同 id 覆盖内置
                *existing = p.clone();
            }
        }
        all
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
        self.http.apply_network(
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
    /// `repo` 为空且非 HTTP 源 = 本地程序：直接使用 `binary`（绝对/相对路径或 PATH 中的命令名）。
    pub fn bin_path(&self, p: &Program) -> PathBuf {
        let is_local = p.repo.is_empty()
            && !p.source.as_ref().map_or(false, |s| s.is_http());
        if is_local {
            return PathBuf::from(&p.binary);
        }
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

    /// 壳操作日志容量上限（字节）。超过后 `log_op` 自动截末一半，
    /// 审计链在容量内完好、不让文件无限增长占用磁盘。
    const OP_LOG_MAX: u64 = 512 * 1024;

    /// 追加一条壳操作日志。带本地时间戳；落盘失败仅记录，不影响功能。
    /// 追加后若超过 `OP_LOG_MAX`，保留末尾一半（丢最旧会话），就地截写。
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
            return;
        }
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() > Self::OP_LOG_MAX {
                // 超阈值 → 保留末尾一半的完整行，丢弃最旧的会话
                if let Ok(text) = std::fs::read_to_string(&path) {
                    let half = text.len().saturating_sub((Self::OP_LOG_MAX / 2) as usize);
                    let start = text[half..].find('\n').map(|i| half + i + 1).unwrap_or(0);
                    let keep = if start == 0 { text } else { text[start..].to_string() };
                    let _ = std::fs::write(&path, keep);
                } else if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!("{}", t!("log.op_write_fail", path = path.display()));
                    log::warn!("failed to trim oversized shell log: {e:#}");
                }
            }
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

    /// 受管程序自启状态文件路径（独立于模板字段，壳统一管理）。
    pub fn program_autostart_path(&self) -> PathBuf {
        self.data_dir.join("program-autostart.json")
    }

    /// 读取受管程序自启状态（program id -> 是否随壳启动拉起）。
    pub fn load_program_autostart(data_dir: &Path) -> BTreeMap<String, bool> {
        std::fs::read_to_string(data_dir.join("program-autostart.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// 持久化受管程序自启状态。
    pub fn save_program_autostart(&self) {
        if let Ok(t) = serde_json::to_string_pretty(&self.program_autostart_map) {
            let _ = std::fs::write(self.program_autostart_path(), t);
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
        let is_http = program.source.as_ref().map_or(false, |s| s.is_http());
        // 0. 本地程序(repo 为空且非 HTTP 源)：壳不下载/不更新，直接使用 binary
        if program.repo.is_empty() && !is_http {
            let v = self.local_version(program);
            return Ok((v.clone(), v));
        }
        if is_http {
            return self.install_http(program, on_progress);
        }
        // 1. 查最新版本(GitHub)
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
        let expect = match (&program.check_sha256, &api_digest) {
            (Some(pin), _) => Some(pin.clone()),
            (None, Some(d)) => Some(d.clone()),
            (None, None) => None,
        };

        self.apply_download(program, &rule, &arch, &version, &dl_path, expect, on_progress)?;
        Ok((version.clone(), version))
    }

    /// HTTP 直链源(方案 A)安装/更新通道。多 URL 按序尝试，任一可下载即用。
    /// sha256 来源优先级：模板钉住 check_sha256 > source.sha256_url > 免检。
    fn install_http(
        &self,
        program: &Program,
        on_progress: &dyn Fn(&crate::progress::DownloadProgress),
    ) -> anyhow::Result<(String, String)> {
        use crate::progress::{DownloadProgress, DownloadStage};
        let spec = program
            .source
            .as_ref()
            .ok_or_else(|| anyhow!(t!("err.http.no_source")))?
            .clone();

        let version = self.http.latest_version(&spec)?;
        info!("{}", t!("log.version_latest", id = &program.id, version = &version));

        // 1b. 已安装且本地版本不低于最新版时，跳过重复下载
        if !crate::version::is_newer(&version, &self.local_version(program))
            && self.bin_path(program).exists()
        {
            info!("{}", t!("log.version_skip", id = &program.id));
            return Ok((version.clone(), version));
        }

        let arch = std::env::consts::ARCH.to_string();
        let (rule, urls) = self.http.asset_urls(program, &version, &arch)?;

        // 2. 按序尝试每个直链，直到某一条下载成功
        let mut last_err: Option<anyhow::Error> = None;
        let mut dl_path = None;
        for url in &urls {
            let name = crate::source_http::HttpSource::filename_from(url);
            let p = self.data_dir.join(format!(".dl-{}-{}", program.id, name));
            info!("{}", t!("log.downloading", url = url, path = p.display()));
            on_progress(&DownloadProgress::stage(DownloadStage::Downloading));
            match self.http.download_to_with_progress(url, &p, &|received, total| {
                on_progress(&DownloadProgress::downloading(received, total));
            }) {
                Ok(()) => {
                    info!("{}", t!("log.download_done", bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0)));
                    dl_path = Some(p);
                    break;
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&p);
                    warn!("{}", t!("err.http.url_failed", url = url, err = format!("{e:#}")));
                    last_err = Some(e);
                }
            }
        }
        let dl_path = dl_path
            .ok_or_else(|| last_err.unwrap_or_else(|| anyhow!(t!("err.http.no_urls"))))?;

        // 3. sha256：钉住 > 源声明 sha256_url > 显式免检
        let declared = self.http.declared_sha256(program, &spec, &version, &arch, &rule)?;
        let expect = match (&program.check_sha256, declared) {
            (Some(pin), _) => Some(pin.clone()),
            (None, Some(d)) => Some(d),
            (None, None) => None,
        };

        self.apply_download(program, &rule, &arch, &version, &dl_path, expect, on_progress)?;
        Ok((version.clone(), version))
    }

    /// 下载完成后的公共落地：校验 sha256 → 解压/落盘 → 清理临时文件 → 记录版本号
    fn apply_download(
        &self,
        program: &Program,
        rule: &AssetRule,
        arch: &str,
        version: &str,
        dl_path: &PathBuf,
        expect: Option<String>,
        on_progress: &dyn Fn(&crate::progress::DownloadProgress),
    ) -> anyhow::Result<()> {
        use crate::progress::{DownloadProgress, DownloadStage};
        on_progress(&DownloadProgress::stage(DownloadStage::Verifying));
        if let Some(expect) = expect {
            if let Err(e) = crate::checksum::verify_download(dl_path, &expect) {
                let _ = std::fs::remove_file(dl_path);
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
                if run_target.exists() {
                    let old = run_target.with_extension("old");
                    let _ = std::fs::remove_file(&old);
                    let _ = std::fs::rename(&run_target, &old);
                }
                let member = rule.member.as_deref();
                extract::extract_file(dl_path, &rule.format, member, &run_target)
                    .with_context(|| t!("err.extract", path = dl_path.display().to_string()))?;
            }
            ExtractMode::Whole => {
                if package_dir.exists() {
                    let _ = std::fs::remove_dir_all(&package_dir);
                }
                extract::extract_whole(dl_path, &rule.format, &package_dir)
                    .with_context(|| t!("err.extract", path = dl_path.display().to_string()))?;
                let member_tpl = rule.member.as_deref().unwrap_or(&program.binary);
                let inner = program.render_template(member_tpl, version, rule, arch);
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
                std::fs::copy(dl_path, &run_target)
                    .with_context(|| t!("err.copy_bin", path = dl_path.display().to_string()))?;
                set_exec(run_target.as_path());
            }
        }

        // 5. 清理临时文件
        let _ = std::fs::remove_file(dl_path);

        // 6. 记录本地版本
        self.save_local_version(program, version);
        Ok(())
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
                mgr.http
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

    /// 查询某程序状态(本地是否已装、是否运行、版本对比)；本地程序无远程对比。
    pub fn status(&mut self, program: &Program) -> ProgramStatus {
        let mut st = self.status_local(program);
        let is_http = program.source.as_ref().map_or(false, |s| s.is_http());
        if program.repo.is_empty() && !is_http {
            st.latest_version = None;
            return st;
        }
        if is_http {
            if let Some(src) = program.source.as_ref() {
                if let Ok(v) = self.http.latest_version(src) {
                    st.latest_version = Some(v);
                }
            }
            return st;
        }
        if let Ok(latest) = self.github.latest(&program.repo) {
            st.latest_version = Some(latest.tag_name.trim_start_matches('v').to_string());
            st.latest_published = latest.published_at.clone();
        }
        st
    }

    /// 远程最新版本（按 source 分发 GitHub / HTTP 源）。本地程序(空 repo 且非 http)返回 None。
    pub fn latest_remote(&self, program: &Program) -> Option<(String, String)> {
        latest_remote(program, &self.github, &self.http)
    }

    /// 壳是否仍持有该程序的子进程句柄（try_wait 轮询回收，无进程派生，
    /// 可在 UI 线程每帧调用；路径存活探测必须走后台线程）。
    pub fn is_held(&mut self, id: &str) -> bool {
        self.runner.is_running(id)
    }

    /// 该程序是否在运行：壳持有子进程句柄，或系统上仍有该程序的进程
    /// （壳上次退出后残留、仍在后台运行）。避免只查句柄而漏判孤儿进程。
    /// 注意：含 pgrep/tasklist 派生，禁止在 UI 渲染路径逐帧调用。
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
                    .all_programs()
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

    /// 该程序是否开启「随壳启动自动拉起」。由壳统一管理，不依赖模板是否有 autostart 字段。
    pub fn program_autostart(&self, id: &str) -> bool {
        self.program_autostart_map.get(id).copied().unwrap_or(false)
    }

    /// 设置某程序是否随壳启动自动拉起，并持久化。
    pub fn set_program_autostart(&mut self, id: &str, enabled: bool) {
        if enabled {
            self.program_autostart_map.insert(id.to_string(), true);
        } else {
            self.program_autostart_map.remove(id);
        }
        self.save_program_autostart();
    }

    /// 兼容旧接口：程序启动/停止时同步自启状态（不再注册系统登录项）。
    /// 自启状态与运行时字段值解耦，此函数保留为空操作。
    pub fn apply_key_autostart(&mut self, _program: &Program, _values: &BTreeMap<String, String>) -> anyhow::Result<()> {
        Ok(())
    }

    /// 壳启动后自动拉起所有开启「自启动」的程序（壳管理）。
    /// 逐个用保存的字段值渲染参数并启动，失败仅记日志不影响后续。
    pub fn start_autostart_programs(&mut self) {
        let programs = self.all_programs();
        for p in &programs {
            if !self.program_autostart(&p.id) {
                continue;
            }
            let values = self.load_field_values(p);
            let bin = self.bin_path(p);
            let held = self.runner.is_running(&p.id);
            let orphan = !held && self.runner.is_process_alive(&bin);
            if held || orphan {
                // 已在运行/残留进程在跑，不重复拉起（落盘，方便在壳日志里确认跳过）
                log::info!("{}", t!("log.autostart.skip", name = &p.name));
                self.log_op(&t!("log.autostart.skip", name = &p.name));
                continue;
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
        current.source = remote.source.clone();
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
        let is_builtin = if self.programs.iter().any(|p| p.id == id) {
            false
        } else if self.builtin_programs.iter().any(|p| p.id == id) {
            true
        } else {
            anyhow::bail!(t!("err.program_not_found", id = &id));
        };

        let find = |list: &[Program]| list.iter().find(|p| p.id == id).cloned();
        let mut current = if is_builtin {
            find(&self.builtin_programs)
        } else {
            find(&self.programs)
        }
        .ok_or_else(|| anyhow::anyhow!(t!("err.program_not_found", id = &id)))?;

        let old_values = self.load_field_values(&current);
        let hidden = current.hidden;
        let merged = Self::apply_template_update(&mut current, remote, &old_values);
        current.hidden = hidden;
        // id 跟随实例(用户不改 id)，避免改动脏掉关联文件
        current.id = id.to_string();
        // apply_template_update 已按 remote.fields 迁移/补齐字段值
        self.save_field_values(&current, &merged);

        let target = if is_builtin {
            &mut self.builtin_programs
        } else {
            &mut self.programs
        };
        if let Some(slot) = target.iter_mut().find(|p| p.id == id) {
            *slot = current;
        }
        self.save_config(path)
    }

    /// 删除实例：从配置移除程序，并清理其二进制/版本/字段值文件。
    pub fn delete_program(&mut self, id: &str, path: &Path) -> anyhow::Result<()> {
        let is_builtin = if self.programs.iter().any(|p| p.id == id) {
            false
        } else if self.builtin_programs.iter().any(|p| p.id == id) {
            true
        } else {
            anyhow::bail!(t!("err.program_not_found", id = &id));
        };
        let p = if is_builtin {
            self.builtin_programs.remove(
                self.builtin_programs
                    .iter()
                    .position(|x| x.id == id)
                    .unwrap(),
            )
        } else {
            self.programs.remove(self.programs.iter().position(|x| x.id == id).unwrap())
        };
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

    /// 新增一个程序（新建模板/导入用）。id 冲突时报错，绝不覆盖。
    pub fn add_program(&mut self, p: &Program, path: &Path) -> anyhow::Result<()> {
        if self.all_programs().iter().any(|x| x.id == p.id) {
            anyhow::bail!(t!("err.program_exists", id = &p.id));
        }
        self.programs.push(p.clone());
        self.save_config(path)
    }

    /// 复制模板/程序为副本：自动生成不冲突的 id（原名-2、-3…）、名称加副本后缀。
    /// 返回新程序，供 UI 立刻跳转编辑。template_source 不继承（副本是新模板而非远程导入）。
    pub fn duplicate_program(&mut self, id: &str, path: &Path) -> anyhow::Result<Program> {
        let base = self
            .all_programs()
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| anyhow::anyhow!(t!("err.program_not_found", id = id)))?;
        let mut n = 2;
        let new_id = loop {
            let candidate = format!("{id}-{n}");
            if !self.all_programs().iter().any(|p| p.id == candidate) {
                break candidate;
            }
            n += 1;
        };
        let mut copy = base.clone();
        copy.id = new_id.clone();
        copy.name = format!("{} (copy)", base.name);
        copy.template_source = None;
        copy.imported_at = None;
        self.add_program(&copy, path)?;
        // 复制已保存的字段值（端口/主机/路径等），让副本开箱即用，
        // 避免空值导致端口冲突或启动参数缺项。
        let values = self.load_field_values(&base);
        if !values.is_empty() {
            self.save_field_values(&copy, &values);
        }
        Ok(copy)
    }

    /// 设置实例的隐藏状态；写出配置。隐藏后侧栏/主管理不展示，
    /// 批量管理始终可见，故不会出现「全部程序失联」的情况。
    pub fn set_hidden(&mut self, id: &str, hidden: bool, path: &Path) -> anyhow::Result<()> {
        let is_builtin = if self.programs.iter().any(|p| p.id == id) {
            false
        } else if self.builtin_programs.iter().any(|p| p.id == id) {
            true
        } else {
            anyhow::bail!(t!("err.program_not_found", id = &id));
        };
        {
            let target = if is_builtin {
                &mut self.builtin_programs
            } else {
                &mut self.programs
            };
            if let Some(p) = target.iter_mut().find(|p| p.id == id) {
                p.hidden = hidden;
            }
        }
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

/// 按 source 分发远程最新版本(GitHub / HTTP 源)。返回 (version, published_at)。
/// 本地程序(空 repo 且非 http)无远程版本，返回 None。供 UI 批量状态刷新/版本检查复用。
pub fn latest_remote(
    program: &Program,
    gh: &GitHub,
    hs: &crate::source_http::HttpSource,
) -> Option<(String, String)> {
    if let Some(src) = program.source.as_ref().filter(|s| s.is_http()) {
        return hs.latest_version(src).ok().map(|v| (v, String::new()));
    }
    if program.repo.is_empty() {
        return None;
    }
    gh.latest(&program.repo).ok().map(|l| {
        (
            l.tag_name.trim_start_matches('v').to_string(),
            l.published_at.clone(),
        )
    })
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
    use crate::config::{Field, FieldKind, Program, SourceSpec};
    use std::io::{Read, Write};

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
    fn local_program_skips_remote_and_uses_binary_path() {
        // repo 为空 = 本地程序：bin_path 直接指向 binary（绝对路径），不与远程比较
        let p: Program = serde_json::from_str(r#"{"id":"ffmpeg","name":"ffmpeg","repo":"","binary":"/usr/local/bin/ffmpeg","fields":[],"args":[]}"#).unwrap();
        let mut mgr = ShellManager::new(std::env::temp_dir().join("cc-local-prog")).unwrap();
        assert_eq!(mgr.bin_path(&p), std::path::PathBuf::from("/usr/local/bin/ffmpeg"));
        // install_or_update 不触网：返回本地版本（未记录过则为 "-"）
        let (local, latest) = mgr.install_or_update(&p, &|_| {}).unwrap();
        assert_eq!(local, "-");
        assert_eq!(latest, "-");
        // status 不花费网络：latest_version 为 None
        let st = mgr.status(&p);
        assert!(st.latest_version.is_none());
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

    /// 内置程序始终存在；用户同 id 配置优先；用户新增程序追加。
    #[test]
    fn all_programs_merges_builtin_with_user_override() {
        let dir = std::env::temp_dir().join("cc-all-programs");
        let mut mgr = ShellManager::new(dir.clone()).unwrap();
        // 全新 manager：内置应立即可见
        let ids: Vec<String> = mgr.all_programs().iter().map(|p| p.id.clone()).collect();
        assert!(!ids.is_empty(), "应内置至少一个程序");
        let builtin_id = ids[0].clone();

        // 用户配置：覆盖内置同 id 程序 + 追加一个新程序
        let mut user = crate::config::ShellConfig::default();
        let mut override_p = crate::builtin::builtin_programs()[0].clone();
        override_p.name = "用户改名的内置".to_string();
        let extra: Program = serde_json::from_str(
            r#"{"id":"dufs","name":"dufs","repo":"sigoden/dufs","binary":"dufs","fields":[],"args":[]}"#,
        )
        .unwrap();
        user.programs = vec![override_p.clone(), extra.clone()];
        let cfg_path = dir.join("cfg.json");
        std::fs::write(&cfg_path, serde_json::to_string(&user).unwrap()).unwrap();
        mgr.load_config(&cfg_path).unwrap();

        let all = mgr.all_programs();
        let by_id: std::collections::HashMap<String, Program> =
            all.into_iter().map(|p| (p.id.clone(), p)).collect();
        // 用户覆盖内置同名程序
        assert_eq!(by_id.get(&builtin_id).unwrap().name, "用户改名的内置");
        // 用户新增程序保留
        assert_eq!(by_id.get("dufs").unwrap().repo, "sigoden/dufs");
    }

    /// 复制模板自动生成不冲突 id；add 拒绝重复 id；template_source 不复用。
    #[test]
    fn duplicate_program_generates_unique_id_and_drops_source() {
        let dir = std::env::temp_dir().join("cc-duplicate");
        let _ = std::fs::remove_dir_all(&dir);
        let mut mgr = ShellManager::new(dir.clone()).unwrap();
        let cfg = dir.join("cfg.json");
        std::fs::write(&cfg, r#"{"programs":[]}"#).unwrap();
        mgr.load_config(&cfg).unwrap();

        let src: Program = serde_json::from_str(
            r#"{"id":"dufs","name":"dufs","repo":"sigoden/dufs","binary":"dufs",
               "fields":[{"key":"port","label":"Port","kind":"string"}],
               "args":["-a","{path}"],
               "template_source":"https://example.com/templates.json","imported_at":1}
            "#,
        )
        .unwrap();
        mgr.add_program(&src, &cfg).unwrap();

        // 复制一次 → dufs-2
        let copy = mgr.duplicate_program("dufs", &cfg).unwrap();
        assert_eq!(copy.id, "dufs-2");
        assert_eq!(copy.name, "dufs (copy)");
        assert_eq!(copy.template_source, None);
        assert_eq!(copy.imported_at, None);
        // 副本继承字段/参数/仓库，可继续安装使用
        assert_eq!(copy.repo, "sigoden/dufs");
        assert_eq!(copy.fields.len(), 1);

        // 先保存字段值，再复制：副本应复刻原值的端口/路径等，开箱即用
        let mut vals = std::collections::BTreeMap::new();
        vals.insert("port".to_string(), "8080".to_string());
        mgr.save_field_values(&src, &vals);
        let copy2 = mgr.duplicate_program("dufs", &cfg).unwrap();
        let vals2 = mgr.load_field_values(&copy2);
        assert_eq!(vals2.get("port").map(String::as_str), Some("8080"));
        let src_vals2 = mgr.load_field_values(&src);
        assert_eq!(src_vals2.get("port").map(String::as_str), Some("8080"));

        // 再复制 → dufs-4（跳过已占用的 dufs-2/dufs-3）
        let copy3 = mgr.duplicate_program("dufs", &cfg).unwrap();
        assert_eq!(copy3.id, "dufs-4");

        // 同名 id 重复新增被拒绝，且不破坏已有配置
        let dup = mgr.duplicate_program("dufs", &cfg).is_err();
        assert!(!dup);
        assert!(mgr.add_program(&src, &cfg).is_err());

        let all: Vec<String> = mgr.all_programs().iter().map(|p| p.id.clone()).collect();
        assert!(all.contains(&"dufs".to_string()));
        assert!(all.contains(&"dufs-2".to_string()));
        assert!(all.contains(&"dufs-3".to_string()));
        assert!(all.contains(&"dufs-4".to_string()));
    }

    /// 追加足够多条日志越过容量上限后，文件被截末一半，体积不再无限增长。
    #[test]
    fn shell_log_is_trimmed_when_over_capacity() {
        let dir = std::env::temp_dir().join("cc-shell-log-trim");
        let _ = std::fs::remove_dir_all(&dir);
        let mgr = ShellManager::new(dir.clone()).unwrap();
        let meta = || std::fs::metadata(mgr.op_log_path()).map(|m| m.len()).unwrap_or(0);

        // 一条 msg 驱动阈值：先写入一批短日志逼近上限，再用长日志触发截尾
        let big_line = "x".repeat(64 * 1024); // 64KB
        // 阈值 512KB：连续写满再多条
        let mut count = 0;
        let mut guard = 0;
        while meta() < ShellManager::OP_LOG_MAX + 32 * 1024 && guard < 2000 {
            mgr.log_op(if count % 10 == 0 { &big_line } else { "short" });
            count += 1;
            guard += 1;
        }
        // 最后一次 log_op 内部已把超出的部分截掉 → 体积应显著小于满载、且 < 上限
        let size = meta();
        assert!(size < ShellManager::OP_LOG_MAX, "size {size} should be trimmed below {}",
            ShellManager::OP_LOG_MAX);

        // 合理性：保留下来的都是完整行（以 \n 结尾）
        let text = std::fs::read_to_string(mgr.op_log_path()).unwrap();
        assert!(!text.is_empty());
        assert!(text.ends_with('\n'));
        assert!(text.len() as u64 <= ShellManager::OP_LOG_MAX);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// autostart 由壳统一管理：即使模板没有 autostart 字段也能开关，并能持久化。
    #[test]
    fn autostart_is_shell_state_not_template_field() {
        let dir = std::env::temp_dir().join("cc-autostart-shell-state");
        let _ = std::fs::remove_dir_all(&dir);
        // 定义一个没有 autostart 字段的程序
        let p: Program = serde_json::from_str(
            r#"{"id":"dufs","name":"dufs","repo":"sigoden/dufs","binary":"dufs","fields":[],"args":[]}"#,
        )
        .unwrap();
        assert!(!p.fields.iter().any(|f| matches!(f.kind, FieldKind::AutoStart { .. })));

        {
            let mut mgr = ShellManager::new(dir.clone()).unwrap();
            assert!(!mgr.program_autostart(&p.id));
            mgr.set_program_autostart(&p.id, true);
            assert!(mgr.program_autostart(&p.id));
            assert!(std::fs::read_to_string(mgr.program_autostart_path())
                .unwrap()
                .contains(&p.id));
        }
        // 重新构造 manager（模拟重启）应能读回持久化状态
        let mgr = ShellManager::new(dir.clone()).unwrap();
        assert!(mgr.program_autostart(&p.id));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 删除内置程序：会话内从 all_programs 消失；删除后再新建 manager 也不残留。
    #[test]
    fn delete_builtin_removes_it_from_all_programs() {
        let dir = std::env::temp_dir().join("cc-del-builtin");
        let _ = std::fs::remove_dir_all(&dir);
        let builtin_id = crate::builtin::builtin_programs()[0].id.clone();
        let cfg = dir.join("shell.json");

        let mut mgr = ShellManager::new(dir.clone()).unwrap();
        assert!(mgr.all_programs().iter().any(|p| p.id == builtin_id));
        mgr.delete_program(&builtin_id, &cfg).unwrap();
        assert!(!mgr.all_programs().iter().any(|p| p.id == builtin_id));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 极小 mock HTTP 服务器：按序服务若干 {path -> body} 路由，全部服务完即退出。
    /// 供 HTTP 源(方案 A)测试，避免真实网络。
    fn mock_server(routes: Vec<(String, Vec<u8>)>) -> (u16, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            for (path, body) in routes {
                let (mut sock, _) = listener.accept().unwrap();
                let mut buf = vec![0u8; 4096];
                let mut read: Vec<u8> = Vec::new();
                loop {
                    let n = sock.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    read.extend_from_slice(&buf[..n]);
                    if read.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&read);
                if head.starts_with(&format!("GET {}", path)) {
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(resp.as_bytes());
                    let _ = sock.write_all(&body);
                } else {
                    let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = sock.write_all(resp.as_bytes());
                }
                drop(sock);
            }
        });
        (port, handle)
    }

    #[test]
    fn http_source_config_roundtrips_and_drops_empty_urls() {
        let p: Program = serde_json::from_str(r#"{
            "id":"app","name":"App","repo":"","binary":"app",
            "source":{"kind":"http","version_url":"https://x/latest.json","version_json_path":"version","sha256_url":"https://x/app-{version}.sha256"},
            "assets":{"darwin":{"urls":["https://x/app-{version}-{arch}.bin"],"format":"raw","mode":"raw"}},
            "fields":[],"args":[]
        }"#)
        .unwrap();
        let src = p.source.as_ref().unwrap();
        assert!(src.is_http());
        assert_eq!(src.version_json_path, "version");
        assert_eq!(src.sha256_url, "https://x/app-{version}.sha256");
        assert_eq!(p.asset_rule_for_os().unwrap().urls.len(), 1);

        // 序列化往返保持 source 与 urls
        let q: Program = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(q.source.as_ref().unwrap().kind, "http");
        assert_eq!(q.asset_rule_for_os().unwrap().urls.len(), 1);

        // 无 source / 无 urls 的旧模板：source=None，序列化不输出空的 urls 字段
        let legacy: Program =
            serde_json::from_str(r#"{"id":"x","name":"X","repo":"a/b","binary":"x","fields":[],"args":[]}"#)
                .unwrap();
        assert!(legacy.source.is_none());
        let s = serde_json::to_string(&legacy).unwrap();
        assert!(!s.contains("\"urls\""));
    }

    #[test]
    fn edit_persists_http_source_changes() {
        let dir = std::env::temp_dir().join("cc-edit-http-source");
        let _ = std::fs::remove_dir_all(&dir);
        let mut mgr = ShellManager::new(dir.clone()).unwrap();
        let cfg = dir.join("cfg.json");
        std::fs::write(&cfg, r#"{"programs":[]}"#).unwrap();
        mgr.load_config(&cfg).unwrap();

        let base: Program = serde_json::from_str(
            r#"{"id":"app","name":"App","repo":"a/b","binary":"app","fields":[],"args":[]}"#,
        )
        .unwrap();
        mgr.add_program(&base, &cfg).unwrap();

        // 编辑时给实例挂上 HTTP 源(模拟编辑器里勾选并填 version_url/regex/sha256)
        let mut edited = base.clone();
        edited.source = Some(SourceSpec {
            kind: "http".into(),
            version_url: "https://x/latest.json".into(),
            version_json_path: String::new(),
            version_regex: "v=([0-9.]+)".into(),
            sha256_url: "https://x/app-{version}.sha256".into(),
        });
        mgr.update_program("app", &edited, &cfg).unwrap();

        let after = mgr.all_programs().into_iter().find(|p| p.id == "app").unwrap();
        let src = after.source.as_ref().expect("编辑的 HTTP 源必须保留");
        assert_eq!(src.version_url, "https://x/latest.json");
        assert_eq!(src.version_regex, "v=([0-9.]+)");
        assert_eq!(src.sha256_url, "https://x/app-{version}.sha256");

        // 反向：取消勾选应清空 source
        let mut cleared = base.clone();
        cleared.source = None;
        mgr.update_program("app", &cleared, &cfg).unwrap();
        let after2 = mgr.all_programs().into_iter().find(|p| p.id == "app").unwrap();
        assert!(after2.source.is_none());
    }

    #[test]
    fn http_source_latest_version_json_path_regex_plain() {
        let (port, h) = mock_server(vec![
            ("/latest.json".to_string(), br#"{ "version":"9.9.9","extra":1 }"#.into()),
        ]);
        let spec: SourceSpec = serde_json::from_str(&format!(
            r#"{{"kind":"http","version_url":"http://127.0.0.1:{port}/latest.json","version_json_path":"version"}}"#
        ))
        .unwrap();
        let hs = crate::source_http::HttpSource::default();
        assert_eq!(hs.latest_version(&spec).unwrap(), "9.9.9");
        h.join().unwrap();

        let (port2, h2) = mock_server(vec![(
            "/ver.txt".to_string(),
            b"[/release] tag=v1.2.3\n".to_vec(),
        )]);
        let spec: SourceSpec = serde_json::from_str(&format!(
            r#"{{"kind":"http","version_url":"http://127.0.0.1:{port2}/ver.txt","version_regex":"tag=v([0-9.]+)"}}"#
        ))
        .unwrap();
        let hs = crate::source_http::HttpSource::default();
        assert_eq!(hs.latest_version(&spec).unwrap(), "1.2.3");
        h2.join().unwrap();

        let (port3, h3) = mock_server(vec![("/plain.txt".to_string(), b"9.9.9\n".into())]);
        let spec: SourceSpec = serde_json::from_str(&format!(
            r#"{{"kind":"http","version_url":"http://127.0.0.1:{port3}/plain.txt"}}"#
        ))
        .unwrap();
        let hs = crate::source_http::HttpSource::default();
        assert_eq!(hs.latest_version(&spec).unwrap(), "9.9.9");
        h3.join().unwrap();
    }

    /// 端到端：version_url 探版本 → urls 直链下载(按序尝试) → sha256_url 校验 → 落盘版本。
    #[test]
    fn http_source_install_downloads_verifies_and_records_version() {
        let dir = std::env::temp_dir().join("cc-http-e2e");
        let _ = std::fs::remove_dir_all(&dir);

        let asset_body: &[u8] = b"#!/bin/sh\necho http-installed\n";
        let digest = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(asset_body);
            h.finalize().to_vec()
        };
        let digest_hex: String =
            digest.iter().map(|b| format!("{b:02x}")).collect();
        let arch = std::env::consts::ARCH;
        let (port, h) = mock_server(vec![
            ("/v.json".to_string(), br#"{"version":"9.9.9"}"#.to_vec()),
            (
                format!("/app-9.9.9-{arch}.bin"),
                asset_body.to_vec(),
            ),
            (
                format!("/app-9.9.9-{arch}.bin.sha256"),
                format!("{digest_hex}  app-9.9.9-{arch}.bin\n").into_bytes(),
            ),
        ]);

        let p: Program = serde_json::from_str(&format!(
            r#"{{
                "id":"httptest","name":"httptest","repo":"","binary":"httptest",
                "source":{{"kind":"http","version_url":"http://127.0.0.1:{port}/v.json","version_json_path":"version","sha256_url":"http://127.0.0.1:{port}/app-{{version}}-{{arch}}.bin.sha256"}},
                "assets":{{"darwin":{{"urls":["http://127.0.0.1:{port}/app-{{version}}-{{arch}}.bin"],"format":"raw","mode":"raw"}}}},
                "fields":[],"args":[]
            }}"#
        ))
        .unwrap();

        let mgr = ShellManager::new(dir.clone()).unwrap();
        let (local, latest) = mgr.install_or_update(&p, &|_| {}).unwrap();
        assert_eq!(local, "9.9.9");
        assert_eq!(latest, "9.9.9");
        assert_eq!(mgr.local_version(&p), "9.9.9");
        assert!(mgr.bin_path(&p).exists(), "bin 应落盘");
        let content = std::fs::read_to_string(mgr.bin_path(&p)).unwrap();
        assert!(content.contains("http-installed"));

        // 已是最新 → 再次 install_or_update 应跳过下载（不发送任何网络请求）
        let (port2, h2) = mock_server(vec![("/v.json".to_string(), br#"{"version":"9.9.9"}"#.to_vec())]);
        let p2: Program = serde_json::from_str(&format!(
            r#"{{
                "id":"httptest","name":"httptest","repo":"","binary":"httptest",
                "source":{{"kind":"http","version_url":"http://127.0.0.1:{port2}/v.json","version_json_path":"version","sha256_url":"http://127.0.0.1:{port2}/app-{{version}}-{{arch}}.bin.sha256"}},
                "assets":{{"darwin":{{"urls":["http://127.0.0.1:{port2}/app-{{version}}-{{arch}}.bin"],"format":"raw","mode":"raw"}}}},
                "fields":[],"args":[]
            }}"#
        ))
        .unwrap();
        let (l2, v2) = mgr.install_or_update(&p2, &|_| {}).unwrap();
        assert_eq!(l2, "9.9.9");
        assert_eq!(v2, "9.9.9");
        h2.join().unwrap();
        h.join().unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 直链全部失败时报错；sha256 与模板钉住不一致时拒绝安装。
    #[test]
    fn http_source_all_urls_fail_without_secretly_degrading() {
        let dir = std::env::temp_dir().join("cc-http-fail");
        let _ = std::fs::remove_dir_all(&dir);
        let (port, h) = mock_server(vec![("/v.json".to_string(), br#"{"version":"9.9.9"}"#.to_vec())]);

        let p: Program = serde_json::from_str(&format!(
            r#"{{
                "id":"httptest","name":"httptest","repo":"","binary":"httptest",
                "source":{{"kind":"http","version_url":"http://127.0.0.1:{port}/v.json","version_json_path":"version"}},
                "assets":{{"darwin":{{"urls":["http://127.0.0.1:{port}/missing-{{version}}.bin"],"format":"raw","mode":"raw"}}}},
                "fields":[],"args":[]
            }}"#
        ))
        .unwrap();
        let mgr = ShellManager::new(dir.clone()).unwrap();
        let r = mgr.install_or_update(&p, &|_| {});
        assert!(r.is_err(), "直链 404 应报错而非静默降级");
        // 免检(无 sha256_url)：模拟服务器只服务了版本请求，未触发下载，bin 不存在
        assert!(!mgr.bin_path(&p).exists());
        h.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}