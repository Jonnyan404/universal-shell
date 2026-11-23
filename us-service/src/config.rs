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
    /// 安装范围
    #[serde(default)]
    pub scope: Scope,
    /// 是否装载到系统（apply 时跳过 disabled 的项，但配置保留）
    #[serde(default = "default_true")]
    pub enabled: bool,
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
        Ok(cfg)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取 {} 失败", path.display()))?;
        Self::parse(&text)
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("序列化服务配置失败")
    }

    pub fn get(&self, id: &str) -> Option<&Service> {
        self.services.iter().find(|s| s.id == id)
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
    if p.is_absolute() || looks_windows_absolute(p) {
        return p.to_path_buf();
    }
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
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
        assert_eq!(p, PathBuf::from("/etc/ushell/logs/dufs.log"));
    }

    #[test]
    fn log_path_honours_relative_and_absolute_overrides() {
        let rel = Config::parse(
            "[[services]]\nid=\"a\"\ncommand=\"/bin/true\"\nlog=\"a.log\"\n",
        )
        .unwrap();
        assert_eq!(
            rel.services[0].log_path(Path::new("/cfg/services.toml")),
            PathBuf::from("/cfg/a.log")
        );
        let abs = Config::parse(
            "[[services]]\nid=\"a\"\ncommand=\"/bin/true\"\nlog=\"/var/log/a.log\"\n",
        )
        .unwrap();
        assert_eq!(
            abs.services[0].log_path(Path::new("/cfg/services.toml")),
            PathBuf::from("/var/log/a.log")
        );
    }

    #[test]
    fn roundtrips_through_toml() {
        let cfg = Config::parse(SAMPLE).unwrap();
        let again = Config::parse(&cfg.to_toml().unwrap()).unwrap();
        assert_eq!(again.services.len(), 2);
        assert_eq!(again.services[0].args, cfg.services[0].args);
    }
}
