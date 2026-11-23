//! 服务声明模型。
//!
//! 刻意保持「中立」：这里只描述「要跑什么」，完全不关心命令是从模板下载来的、
//! 还是用户自己写的脚本。安装来源属于上层产品，调度属于这里。

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 崩溃后是否重启（对应 launchd 的 KeepAlive、systemd 的 Restart、计划任务的 RestartOnFailure）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Restart {
    /// 退出即结束，不重启
    #[default]
    Never,
    /// 非 0 退出才重启（适合"跑完就退出"的脚本不要重启）
    OnFailure,
    /// 无论如何都重启（守护型）
    Always,
}

/// 安装范围。system 需要相应权限（macOS 需 sudo 写 /Library/LaunchDaemons）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// 用户登录后启动（launchd LaunchAgent / systemd --user / 登录触发）
    #[default]
    User,
    /// 系统级，登录前即可运行（LaunchDaemon / systemd system unit / 系统启动）
    System,
}

fn default_restart_secs() -> u64 {
    10
}

fn default_wait_secs() -> u64 {
    60
}

fn default_true() -> bool {
    true
}

/// 一条服务声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    /// 唯一 id，同时用于文件名与 launchd Label（只允许字母数字、`-`、`_`、`.`）
    pub id: String,
    /// 展示名，缺省用 id
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 可执行文件：绝对路径，或 PATH 上的命令名（建议绝对路径，登录环境 PATH 很短）
    pub command: String,
    /// 参数，原样传给 command（不做 shell 展开）
    #[serde(default)]
    pub args: Vec<String>,
    /// 注入的环境变量（值不写日志）
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// 工作目录
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// 日志文件；缺省 `<配置目录>/logs/<id>.log`
    #[serde(default)]
    pub log: Option<PathBuf>,
    /// 启动前延迟秒数（在包装器里 sleep，三平台语义一致）
    #[serde(default)]
    pub delay_secs: u64,
    /// 崩溃重启策略
    #[serde(default)]
    pub restart: Restart,
    /// 重启间隔秒数（避免崩溃风暴）
    #[serde(default = "default_restart_secs")]
    pub restart_secs: u64,
    /// 依赖的服务 id：本服务会等它们「就绪」之后再启动
    /// （语义同 systemd 的 Wants+After：超时只警告并继续，不会把整条链卡死）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// 等待依赖的总超时（秒）
    #[serde(default = "default_wait_secs")]
    pub wait_timeout_secs: u64,
    /// 就绪判定：本机 TCP 端口可连（优先于其它判定，最贴近「真的能用了」）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_port: Option<u16>,
    /// 就绪判定：文件存在（相对路径基于配置目录）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_file: Option<PathBuf>,
    /// 安装范围
    #[serde(default)]
    pub scope: Scope,
    /// 是否装载到系统（apply 时跳过 disabled 的项，但配置保留）
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 来源标记：`program:<id>` 表示由主应用里某条「程序」派生而来。
    /// 派生条目在同步时会被重建，手改会在下次同步被覆盖。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

impl Service {
    /// 展示名（空则回退 id）
    pub fn title(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }

    /// launchd 的 Label / 计划任务的路径前缀都基于它
    pub fn label(&self) -> String {
        format!("com.ushell.{}", self.id)
    }

    /// 有效日志路径：显式配置优先，否则放在配置目录的 logs/ 下
    pub fn log_path(&self, config_path: &Path) -> PathBuf {
        match &self.log {
            Some(p) if p.is_absolute() => p.clone(),
            Some(p) => config_dir(config_path).join(p),
            None => config_dir(config_path)
                .join("logs")
                .join(format!("{}.log", self.id)),
        }
    }

    /// 校验：id 会变成文件名与 Label，必须能安全落盘
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            bail!("service id 不能为空");
        }
        if !self
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            bail!("service id `{}` 只能包含字母数字与 - _ .", self.id);
        }
        if self.id.starts_with('.') {
            bail!("service id `{}` 不能以 . 开头", self.id);
        }
        if self.command.trim().is_empty() {
            bail!("service `{}` 缺少 command", self.id);
        }
        Ok(())
    }
}

/// 配置文件（TOML，`[[services]]` 数组）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub services: Vec<Service>,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        let cfg: Config = toml::from_str(text).context("解析服务配置失败")?;
        let mut seen = std::collections::BTreeSet::new();
        for s in &cfg.services {
            s.validate()?;
            if !seen.insert(s.id.clone()) {
                bail!("service id `{}` 重复", s.id);
            }
        }
        cfg.validate_graph()?;
        Ok(cfg)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取 {} 失败", path.display()))?;
        Self::parse(&text)
    }

    /// 校验依赖图：不能指向不存在的服务、不能自依赖、不能成环。
    ///
    /// 环是最容易犯又最难查的错（A 等 B、B 等 A），开机后两边都卡到超时；
    /// 所以在加载配置时就拒绝，而不是等到开机才发现。
    pub fn validate_graph(&self) -> Result<()> {
        for s in &self.services {
            for dep in &s.depends_on {
                if dep == &s.id {
                    bail!("service `{}` 不能依赖自己", s.id);
                }
                if !self.services.iter().any(|x| &x.id == dep) {
                    bail!("service `{}` 依赖了不存在的服务 `{}`", s.id, dep);
                }
            }
        }
        // DFS 找环，同时给出具体的环路径便于排查
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            New,
            InStack,
            Done,
        }
        let mut mark: std::collections::BTreeMap<&str, Mark> =
            self.services.iter().map(|s| (s.id.as_str(), Mark::New)).collect();
        let mut stack: Vec<&str> = Vec::new();

        fn visit<'a>(
            id: &'a str,
            cfg: &'a Config,
            mark: &mut std::collections::BTreeMap<&'a str, Mark>,
            stack: &mut Vec<&'a str>,
        ) -> Result<()> {
            match mark.get(id) {
                Some(Mark::Done) => return Ok(()),
                Some(Mark::InStack) => {
                    let at = stack.iter().position(|x| *x == id).unwrap_or(0);
                    let mut cycle: Vec<&str> = stack[at..].to_vec();
                    cycle.push(id);
                    bail!("依赖成环：{}", cycle.join(" → "));
                }
                _ => {}
            }
            mark.insert(id, Mark::InStack);
            stack.push(id);
            if let Some(s) = cfg.services.iter().find(|s| s.id == id) {
                for dep in &s.depends_on {
                    visit(dep, cfg, mark, stack)?;
                }
            }
            stack.pop();
            mark.insert(id, Mark::Done);
            Ok(())
        }

        for s in &self.services {
            visit(&s.id, self, &mut mark, &mut stack)?;
        }
        Ok(())
    }

    /// 拓扑序（依赖在前），用于 `us-svc order` 与界面展示
    pub fn topo_order(&self) -> Result<Vec<&Service>> {
        self.validate_graph()?;
        let mut out: Vec<&Service> = Vec::new();
        let mut done: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        while out.len() < self.services.len() {
            for s in &self.services {
                if done.contains(s.id.as_str()) {
                    continue;
                }
                if s.depends_on.iter().all(|d| done.contains(d.as_str())) {
                    done.insert(s.id.as_str());
                    out.push(s);
                    break;
                }
            }
        }
        Ok(out)
    }

    /// 依赖是否就绪（供包装器等待）
    pub fn dependency_ready(&self, dep: &Service, config_path: &Path) -> bool {
        if let Some(port) = dep.ready_port {
            use std::net::{SocketAddr, TcpStream};
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            return TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).is_ok();
        }
        if let Some(f) = &dep.ready_file {
            let p = if f.is_absolute() { f.clone() } else { config_dir(config_path).join(f) };
            return p.exists();
        }
        // 没声明就绪方式：进程在跑就算就绪
        crate::state(dep, config_path) == crate::ServiceState::Running
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("序列化服务配置失败")
    }

    pub fn get(&self, id: &str) -> Option<&Service> {
        self.services.iter().find(|s| s.id == id)
    }

    /// 新增或按 id 替换（UI 的「保存」走这里）
    pub fn upsert(&mut self, svc: Service) -> Result<()> {
        svc.validate()?;
        match self.services.iter_mut().find(|s| s.id == svc.id) {
            Some(slot) => *slot = svc,
            None => self.services.push(svc),
        }
        Ok(())
    }

    /// 删除一条；返回是否真的删了
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.services.len();
        self.services.retain(|s| s.id != id);
        self.services.len() != before
    }

    /// 写回文件。
    ///
    /// ⚠️ 会重排/丢失原文件里的注释（toml 序列化不保留），
    /// 一旦在界面里改过配置，就以界面/本文件为唯一来源。
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = self.to_toml()?;
        std::fs::write(path, text)
            .with_context(|| format!("写入 {} 失败", path.display()))?;
        Ok(())
    }
}

/// 配置所在目录（日志默认落在这里）。
///
/// **必须返回绝对路径**：调度器（launchd 的 cwd 是 `/`、systemd 的 cwd 是 `/`）
/// 不会继承你当前的 shell 目录，相对路径会让「找不到配置」或「日志写到 /」。
pub fn config_dir(config_path: &Path) -> PathBuf {
    let dir = match config_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    absolute(&dir)
}

/// 转为绝对路径（不要求已存在，不解析符号链接）。
///
/// 跨平台生成时要保留「另一平台」的绝对路径：在 macOS 上给 Windows 生成任务时，
/// `C:\x` 在 `Path::is_absolute()` 眼里是相对路径，直接拼接会毁掉它。
pub fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() || looks_windows_absolute(p) || looks_unix_absolute(p) {
        return p.to_path_buf();
    }
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// `/etc/x` 在 Windows 上 `is_absolute()` 为 false，但它显然不是「相对于当前目录」。
/// 跨平台生成时（在 Windows 上为 Linux 生成 unit）必须原样保留。
fn looks_unix_absolute(p: &Path) -> bool {
    p.to_string_lossy().starts_with('/')
}

fn looks_windows_absolute(p: &Path) -> bool {
    let s = p.to_string_lossy();
    let b = s.as_bytes();
    (b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'/'))
        || s.starts_with("\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[services]]
id = "dufs"
name = "dufs 文件服务"
command = "/usr/local/bin/dufs"
args = ["-p", "5000", "/data"]
delay_secs = 3
restart = "on_failure"

[[services]]
id = "sync"
command = "/bin/sh"
args = ["-c", "echo hi"]
scope = "system"
"#;

    #[test]
    fn parses_sample_and_applies_defaults() {
        let cfg = Config::parse(SAMPLE).unwrap();
        assert_eq!(cfg.services.len(), 2);
        let d = &cfg.services[0];
        assert_eq!(d.title(), "dufs 文件服务");
        assert_eq!(d.restart, Restart::OnFailure);
        assert_eq!(d.scope, Scope::User);
        assert_eq!(d.restart_secs, 10, "缺省重启间隔");
        assert!(d.enabled, "缺省启用");
        let s = &cfg.services[1];
        assert_eq!(s.title(), "sync", "name 为空回退 id");
        assert_eq!(s.scope, Scope::System);
        assert_eq!(s.restart, Restart::Never);
    }

    #[test]
    fn rejects_duplicate_ids() {
        let bad = r#"
[[services]]
id = "a"
command = "/bin/true"
[[services]]
id = "a"
command = "/bin/true"
"#;
        let err = Config::parse(bad).unwrap_err().to_string();
        assert!(err.contains("重复"), "实际错误: {err}");
    }

    #[test]
    fn rejects_ids_that_would_escape_the_directory() {
        for id in ["../evil", "a/b", "", " a", ".hidden"] {
            let text = format!("[[services]]\nid = \"{id}\"\ncommand = \"/bin/true\"\n");
            assert!(Config::parse(&text).is_err(), "`{id}` 应被拒绝");
        }
    }

    #[test]
    fn requires_command() {
        let text = "[[services]]\nid = \"x\"\ncommand = \"\"\n";
        assert!(Config::parse(text).is_err());
    }

    #[test]
    fn log_path_defaults_next_to_config() {
        let cfg = Config::parse(SAMPLE).unwrap();
        let p = cfg.services[0].log_path(Path::new("/etc/ushell/services.toml"));
        assert_eq!(
            p,
            PathBuf::from("/etc/ushell").join("logs").join("dufs.log"),
            "日志默认落在配置目录的 logs/ 下"
        );
    }

    #[test]
    fn log_path_honours_relative_and_absolute_overrides() {
        let rel = Config::parse(
            "[[services]]\nid=\"a\"\ncommand=\"/bin/true\"\nlog=\"a.log\"\n",
        )
        .unwrap();
        assert_eq!(
            rel.services[0].log_path(Path::new("/cfg/services.toml")),
            PathBuf::from("/cfg").join("a.log")
        );
        let abs = Config::parse(
            "[[services]]\nid=\"a\"\ncommand=\"/bin/true\"\nlog=\"/var/log/a.log\"\n",
        )
        .unwrap();
        assert_eq!(
            abs.services[0].log_path(Path::new("/cfg/services.toml")),
            PathBuf::from("/var/log/a.log"),
            "显式绝对路径不该被改写"
        );
    }

    #[test]
    fn upsert_replaces_by_id_and_keeps_order() {
        let mut cfg = Config::parse(SAMPLE).unwrap();
        let mut edited = cfg.services[0].clone();
        edited.name = "改过的名字".into();
        cfg.upsert(edited).unwrap();
        assert_eq!(cfg.services.len(), 2, "同 id 应替换而不是追加");
        assert_eq!(cfg.services[0].name, "改过的名字");
        assert_eq!(cfg.services[1].id, "sync", "其它条目顺序不变");

        let fresh = Service {
            id: "new".into(),
            name: String::new(),
            description: String::new(),
            command: "/bin/true".into(),
            args: vec!["--x".into()],
            env: BTreeMap::new(),
            cwd: None,
            log: None,
            delay_secs: 0,
            restart: Restart::Never,
            restart_secs: 10,
            depends_on: Vec::new(),
            wait_timeout_secs: 60,
            ready_port: None,
            ready_file: None,
            scope: Scope::User,
            enabled: true,
            origin: None,
        };
        cfg.upsert(fresh).unwrap();
        assert_eq!(cfg.services.len(), 3);
        assert_eq!(cfg.services[2].id, "new");
    }

    #[test]
    fn upsert_rejects_bad_id() {
        let mut cfg = Config::parse(SAMPLE).unwrap();
        let mut bad = cfg.services[0].clone();
        bad.id = "../evil".into();
        assert!(cfg.upsert(bad).is_err());
    }

    #[test]
    fn remove_reports_whether_it_matched() {
        let mut cfg = Config::parse(SAMPLE).unwrap();
        assert!(cfg.remove("dufs"));
        assert_eq!(cfg.services.len(), 1);
        assert!(!cfg.remove("dufs"), "第二次删除应返回 false");
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = std::env::temp_dir().join(format!("us-svc-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("services.toml");
        let mut cfg = Config::parse(SAMPLE).unwrap();
        cfg.remove("sync");
        cfg.save(&path).unwrap();
        let again = Config::load(&path).unwrap();
        assert_eq!(again.services.len(), 1);
        assert_eq!(again.services[0].id, "dufs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_dependency_cycles_self_deps_and_unknown_ids() {
        let cyc = r#"
[[services]]
id = "a"
command = "/bin/true"
depends_on = ["b"]
[[services]]
id = "b"
command = "/bin/true"
depends_on = ["a"]
"#;
        let err = Config::parse(cyc).unwrap_err().to_string();
        assert!(err.contains("成环"), "应指出成环而不是等到开机超时: {err}");

        let me = "[[services]]\nid=\"a\"\ncommand=\"/bin/true\"\ndepends_on=[\"a\"]\n";
        assert!(Config::parse(me).unwrap_err().to_string().contains("自己"));

        let missing = "[[services]]\nid=\"a\"\ncommand=\"/bin/true\"\ndepends_on=[\"nope\"]\n";
        assert!(Config::parse(missing).unwrap_err().to_string().contains("不存在"));
    }

    #[test]
    fn topo_order_puts_dependencies_first() {
        let text = r#"
[[services]]
id = "app"
command = "/bin/true"
depends_on = ["db", "cache"]

[[services]]
id = "cache"
command = "/bin/true"

[[services]]
id = "db"
command = "/bin/true"
"#;
        let cfg = Config::parse(text).unwrap();
        let ids: Vec<&str> = cfg.topo_order().unwrap().iter().map(|s| s.id.as_str()).collect();
        let pos = |id: &str| ids.iter().position(|x| *x == id).unwrap();
        assert_eq!(ids.last().copied(), Some("app"), "依赖方应排最后: {ids:?}");
        assert!(pos("db") < pos("app"));
        assert!(pos("cache") < pos("app"));
    }

    #[test]
    fn readiness_defaults_to_process_state_and_honours_declarations() {
        let cfg = Config::parse(
            r#"
[[services]]
id = "a"
command = "/bin/true"
[[services]]
id = "port"
command = "/bin/true"
ready_port = 65000
"#,
        )
        .unwrap();
        let cfg_path = Path::new("/cfg/services.toml");
        // 没声明就绪方式 → 看进程状态（这里没在跑，所以 false）
        assert!(!cfg.dependency_ready(cfg.get("a").unwrap(), cfg_path));
        // 端口没人监听 → false（用高位端口避免撞上真实服务）
        assert!(!cfg.dependency_ready(cfg.get("port").unwrap(), cfg_path));
    }

    #[test]
    fn roundtrips_through_toml() {
        let cfg = Config::parse(SAMPLE).unwrap();
        let again = Config::parse(&cfg.to_toml().unwrap()).unwrap();
        assert_eq!(again.services.len(), 2);
        assert_eq!(again.services[0].args, cfg.services[0].args);
    }
}
