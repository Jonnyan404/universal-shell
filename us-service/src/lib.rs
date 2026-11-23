//! us-service —— 声明式本地服务调度。
//!
//! 设计要点（为什么这样分层）：
//!
//! 1. **真相在配置，系统配置是产物**：`services.toml` 是唯一事实来源，
//!    launchd plist / systemd unit / 计划任务 XML 都是可随时重建的生成物。
//! 2. **调度器调的是包装器，不是目标程序**：`us-svc run --config … --id …`。
//!    日志重定向、env 注入、延迟、退出码记录都在包装器里，三平台语义一致
//!    （Windows 计划任务本身不管 stdout，这层是必需的）。
//! 3. **不关心程序从哪来**：命令可以是下载来的二进制，也可以是用户自己的脚本；
//!    安装来源是上层产品的事，本 crate 只负责"怎么稳定地跑起来"。

pub mod config;
pub mod ui;
pub mod launchd;
pub mod runner;
pub mod schtasks;
pub mod systemd;

pub use config::{config_dir, Config, Restart, Scope, Service};

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// 生成配置时需要的上下文：由谁拉起、配置在哪。
#[derive(Debug, Clone)]
pub struct Ctx {
    /// 包装器可执行文件（通常是 us-svc 自己）
    pub wrapper: PathBuf,
    /// 服务配置文件路径
    pub config: PathBuf,
}

impl Ctx {
    pub fn new(config: &Path) -> Result<Self> {
        let wrapper = std::env::current_exe().context("定位 us-svc 可执行文件失败")?;
        Ok(Self {
            wrapper,
            // 配置路径必须绝对：调度器的工作目录与我们无关
            config: config::absolute(config),
        })
    }

    pub fn with_wrapper(config: &Path, wrapper: impl Into<PathBuf>) -> Self {
        Self {
            wrapper: wrapper.into(),
            config: config::absolute(config),
        }
    }
}

/// 包装器被调度器调起时的完整 argv（三平台共用同一套，避免行为分叉）
pub fn run_argv(ctx: &Ctx, id: &str) -> Vec<String> {
    vec![
        ctx.wrapper.to_string_lossy().into_owned(),
        "run".to_string(),
        "--config".to_string(),
        ctx.config.to_string_lossy().into_owned(),
        "--id".to_string(),
        id.to_string(),
    ]
}

/// 一条待落盘的产物
#[derive(Debug, Clone)]
pub struct PlanItem {
    pub service_id: String,
    pub scope: Scope,
    /// 目标路径（systemd/launchd 是单元文件；Windows 是待导入的 XML）
    pub path: PathBuf,
    pub content: String,
}

/// 当前平台的产物清单
pub fn plan(cfg: &Config, ctx: &Ctx) -> Result<Vec<PlanItem>> {
    plan_for(cfg, ctx, Platform::current())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Launchd,
    Systemd,
    Schtasks,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::Launchd
        } else if cfg!(target_os = "windows") {
            Platform::Schtasks
        } else {
            Platform::Systemd
        }
    }
}

/// 指定平台生成产物（便于跨平台测试与「导出给别人用」）
pub fn plan_for(cfg: &Config, ctx: &Ctx, p: Platform) -> Result<Vec<PlanItem>> {
    let mut out = Vec::new();
    for s in cfg.services.iter().filter(|s| s.enabled) {
        s.validate()?;
        let (path, content) = match p {
            Platform::Launchd => (launchd::unit_path(s), launchd::plist(s, ctx)),
            Platform::Systemd => (systemd::unit_path(s), systemd::unit(s, ctx)),
            Platform::Schtasks => (schtasks::xml_path(s, &ctx.config), schtasks::xml(s, ctx)),
        };
        out.push(PlanItem {
            service_id: s.id.clone(),
            scope: s.scope,
            path,
            content,
        });
    }
    Ok(out)
}

/// 把产物写到指定目录（`gen` 用；不碰系统）
pub fn write_plans(items: &[PlanItem], out_dir: &Path) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("创建输出目录 {} 失败", out_dir.display()))?;
    let mut written = Vec::new();
    for it in items {
        let name = it
            .path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_else(|| format!("{}.conf", it.service_id).into());
        let target = out_dir.join(name);
        std::fs::write(&target, &it.content)
            .with_context(|| format!("写入 {} 失败", target.display()))?;
        written.push(target);
    }
    Ok(written)
}

/// 装载到系统：写单元文件并让调度器接管。
///
/// 只自动处理 **用户级**；系统级涉及 root 权限，返回明确错误让人自己 sudo，
/// 不在程序里偷偷提权。
pub fn install(items: &[PlanItem], cfg: &Config) -> Result<()> {
    for it in items {
        if it.scope == Scope::System {
            bail!(
                "`{}` 是 system 级服务：请以管理员权限把它放到 {} 后手动装载（本工具不自动提权）",
                it.service_id,
                it.path.display()
            );
        }
        if let Some(parent) = it.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建 {} 失败", parent.display()))?;
        }
        std::fs::write(&it.path, &it.content)
            .with_context(|| format!("写入 {} 失败", it.path.display()))?;
        reload(Platform::current(), it, cfg)?;
    }
    Ok(())
}

fn reload(p: Platform, item: &PlanItem, cfg: &Config) -> Result<()> {
    let svc = cfg.get(&item.service_id);
    match p {
        Platform::Launchd => {
            let label = svc.map(Service::label).unwrap_or_default();
            // 先卸载旧的（可能不存在），再装载；bootstrap 比 load 更能反映错误
            let _ = std::process::Command::new("launchctl")
                .args(["bootout", &format!("gui/{}", current_uid()), &label])
                .output();
            let out = std::process::Command::new("launchctl")
                .args([
                    "bootstrap",
                    &format!("gui/{}", current_uid()),
                    &item.path.to_string_lossy(),
                ])
                .output()
                .context("执行 launchctl bootstrap 失败")?;
            if !out.status.success() {
                bail!(
                    "launchctl bootstrap 失败: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
        }
        Platform::Systemd => {
            run_ok("systemctl", &["--user", "daemon-reload"])?;
            run_ok("systemctl", &["--user", "enable", "--now", &item.service_id])?;
        }
        Platform::Schtasks => {
            let name = schtasks::task_name(&item.service_id);
            run_ok(
                "schtasks",
                &[
                    "/Create",
                    "/TN",
                    &name,
                    "/XML",
                    &item.path.to_string_lossy(),
                    "/F",
                ],
            )?;
        }
    }
    Ok(())
}

fn run_ok(bin: &str, args: &[&str]) -> Result<()> {
    let out = std::process::Command::new(bin)
        .args(args)
        .output()
        .with_context(|| format!("执行 {bin} 失败（是否未安装？）"))?;
    if !out.status.success() {
        bail!(
            "{bin} {} 失败: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(unix)]
pub fn current_uid() -> u32 {
    // 不引入 libc：读 getuid 的常见替代是解析 `id -u`
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(501)
}

#[cfg(not(unix))]
pub fn current_uid() -> u32 {
    0
}

fn cmd_capture(bin: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// 服务在系统里的实际状态（看板/CLI 共用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    /// 进程正在跑
    Running,
    /// 有单元文件但没在跑（未装载 / 已停止 / 崩溃待拉起）
    Stopped,
    /// 系统里没有这个服务的配置
    NotInstalled,
    /// 查询失败（调度器工具缺失等）
    Unknown,
}

impl ServiceState {
    pub fn label(self) -> &'static str {
        match self {
            ServiceState::Running => "运行中",
            ServiceState::Stopped => "未运行",
            ServiceState::NotInstalled => "未启用",
            ServiceState::Unknown => "未知",
        }
    }
    pub fn is_running(self) -> bool {
        self == ServiceState::Running
    }
}

/// 查询单个服务的实际状态（只读，不提权）
pub fn state(s: &Service, config: &Path) -> ServiceState {
    let installed = match Platform::current() {
        Platform::Launchd => launchd::unit_path(s).exists(),
        Platform::Systemd => systemd::unit_path(s).exists(),
        Platform::Schtasks => schtasks::xml_path(s, &config.to_path_buf()).exists(),
    };
    if !installed {
        return ServiceState::NotInstalled;
    }
    match Platform::current() {
        Platform::Launchd => {
            let label = s.label();
            match cmd_capture("launchctl", &["list", &label]) {
                Some(out) => {
                    let running = launchd_running(&out);
                    if running {
                        ServiceState::Running
                    } else {
                        ServiceState::Stopped
                    }
                }
                // 有文件但 launchctl 不认识：写入过但还没装载
                None => ServiceState::Stopped,
            }
        }
        Platform::Systemd => match cmd_capture("systemctl", &["--user", "is-active", &s.id]) {
            Some(t) if t.starts_with("active") => ServiceState::Running,
            Some(_) => ServiceState::Stopped,
            None => ServiceState::Unknown,
        },
        Platform::Schtasks => {
            let name = schtasks::task_name(&s.id);
            match cmd_capture("schtasks", &["/Query", "/TN", &name, "/FO", "LIST"]) {
                Some(out) => {
                    if out.to_lowercase().contains("running") {
                        ServiceState::Running
                    } else {
                        ServiceState::Stopped
                    }
                }
                None => ServiceState::Unknown,
            }
        }
    }
}


/// 判断 `launchctl list <label>` 的输出里是否真有进程在跑。
///
/// 两种格式都要认（实测 macOS 14 返回字典，旧版本/无参调用返回表格）：
/// ```text
/// { "PID" = 77514; ... }              // 字典
/// PID   Status  Label                // 表格（表头首列是字母，未运行时是 "-"）
/// 77514 0       com.ushell.dufs
/// ```
fn launchd_running(out: &str) -> bool {
    for line in out.lines() {
        if let Some(rest) = line.trim().strip_prefix("\"PID\" = ") {
            let num = rest.trim_end_matches(';').trim();
            return num.parse::<u32>().map(|p| p > 0).unwrap_or(false);
        }
    }
    out.lines().any(|l| {
        l.split_whitespace()
            .next()
            .map(|pid| pid != "-" && pid.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false)
    })
}

/// 端上用的一句话状态（CLI `status` 用）
pub fn probe(s: &Service, config: &Path) -> String {
    state(s, config).label().to_string()
}

/// 从系统卸载：停止并删掉单元文件，配置本身不动
pub fn uninstall(s: &Service, config: &Path) -> Result<()> {
    match Platform::current() {
        Platform::Launchd => {
            let target = format!("gui/{}/{}", current_uid(), s.label());
            let _ = std::process::Command::new("launchctl")
                .args(["bootout", &target])
                .output();
            let path = launchd::unit_path(s);
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("删除 {} 失败", path.display()))?;
            }
        }
        Platform::Systemd => {
            let _ = run_ok("systemctl", &["--user", "disable", "--now", &s.id]);
            let path = systemd::unit_path(s);
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("删除 {} 失败", path.display()))?;
            }
            let _ = run_ok("systemctl", &["--user", "daemon-reload"]);
        }
        Platform::Schtasks => {
            let name = schtasks::task_name(&s.id);
            let _ = run_ok("schtasks", &["/Delete", "/TN", &name, "/F"]);
            let path = schtasks::xml_path(s, &config.to_path_buf());
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("删除 {} 失败", path.display()))?;
            }
        }
    }
    Ok(())
}

/// 启动 / 重启一个已装载的服务
pub fn kick(s: &Service, restart: bool) -> Result<()> {
    match Platform::current() {
        Platform::Launchd => {
            let target = format!("gui/{}/{}", current_uid(), s.label());
            if restart {
                run_ok("launchctl", &["kickstart", "-k", target.as_str()])
            } else {
                run_ok("launchctl", &["kickstart", target.as_str()])
            }
        }
        Platform::Systemd => run_ok(
            "systemctl",
            &["--user", if restart { "restart" } else { "start" }, &s.id],
        ),
        Platform::Schtasks => {
            let name = schtasks::task_name(&s.id);
            if restart {
                let _ = run_ok("schtasks", &["/End", "/TN", &name]);
            }
            run_ok("schtasks", &["/Run", "/TN", &name])
        }
    }
}

/// 停止（不卸载）
pub fn stop(s: &Service) -> Result<()> {
    match Platform::current() {
        Platform::Launchd => {
            let target = format!("gui/{}/{}", current_uid(), s.label());
            run_ok("launchctl", &["kill", "SIGTERM", &target])
        }
        Platform::Systemd => run_ok("systemctl", &["--user", "stop", &s.id]),
        Platform::Schtasks => {
            let name = schtasks::task_name(&s.id);
            run_ok("schtasks", &["/End", "/TN", &name])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx::with_wrapper(
            Path::new("/cfg/services.toml"),
            PathBuf::from("/usr/local/bin/us-svc"),
        )
    }

    fn cfg() -> Config {
        Config::parse(
            r#"
[[services]]
id = "dufs"
command = "/usr/local/bin/dufs"
args = ["-p", "5000"]
enabled = true

[[services]]
id = "off"
command = "/bin/true"
enabled = false
"#,
        )
        .unwrap()
    }

    #[test]
    fn run_argv_is_stable_across_platforms() {
        let argv = run_argv(&ctx(), "dufs");
        assert_eq!(argv[0], "/usr/local/bin/us-svc");
        assert_eq!(&argv[1..], &["run", "--config", "/cfg/services.toml", "--id", "dufs"]);
    }

    #[test]
    fn parses_both_launchctl_output_formats() {
        // 实测 macOS 14 的字典格式
        let dict = r#"{
	"Label" = "com.ushell.dufs";
	"PID" = 77514;
	"LastExitStatus" = 0;
}"#;
        assert!(super::launchd_running(dict));

        let dict_idle = r#"{
	"Label" = "com.ushell.dufs";
	"PID" = 0;
}"#;
        assert!(!super::launchd_running(dict_idle), "PID=0 不算在跑");

        // 旧的表格格式：表头首列是 PID 三个字母，不能被当成分工号
        let table = "PID\tStatus\tLabel\n77514\t0\tcom.ushell.dufs\n";
        assert!(super::launchd_running(table));

        let table_idle = "PID\tStatus\tLabel\n-\t0\tcom.ushell.dufs\n";
        assert!(!super::launchd_running(table_idle));
    }

    #[test]
    fn disabled_services_are_not_planned() {
        let items = plan_for(&cfg(), &ctx(), Platform::Launchd).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].service_id, "dufs");
    }

    #[test]
    fn each_platform_targets_its_own_location() {
        let c = cfg();
        let l = plan_for(&c, &ctx(), Platform::Launchd).unwrap();
        assert!(l[0].path.ends_with("Library/LaunchAgents/com.ushell.dufs.plist"));

        let s = plan_for(&c, &ctx(), Platform::Systemd).unwrap();
        assert!(s[0].path.ends_with(".config/systemd/user/dufs.service"));

        let w = plan_for(&c, &ctx(), Platform::Schtasks).unwrap();
        assert!(w[0].path.ends_with("schtasks/dufs.xml"));
    }

    #[test]
    fn write_plans_lands_files_in_out_dir() {
        let dir = std::env::temp_dir().join(format!("us-svc-plan-{}", std::process::id()));
        let items = plan_for(&cfg(), &ctx(), Platform::Systemd).unwrap();
        let written = write_plans(&items, &dir).unwrap();
        assert_eq!(written.len(), 1);
        assert!(written[0].exists());
        assert!(std::fs::read_to_string(&written[0]).unwrap().contains("ExecStart="));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
