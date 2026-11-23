//! Linux：systemd 用户/系统单元。

use crate::{run_argv, Ctx, Restart, Scope, Service};
use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// 用户级 `~/.config/systemd/user/`，系统级 `/etc/systemd/system/`
pub fn unit_path(s: &Service) -> PathBuf {
    match s.scope {
        Scope::User => home().join(".config/systemd/user"),
        Scope::System => PathBuf::from("/etc/systemd/system"),
    }
    .join(format!("{}.service", s.id))
}

/// systemd 的 ExecStart 解析规则：`%` 是说明符、`$` 是变量替换，都要转义；
/// 含空白或特殊字符时整段加双引号。
fn quote(a: &str) -> String {
    let mut s = a.replace('%', "%%").replace('$', "$$");
    s = s.replace('\\', "\\\\").replace('"', "\\\"");
    if s.is_empty()
        || s.chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | ';' | '&' | '|' | '<' | '>' | '(' | ')' | '*' | '?' | '[' | ']' | '#' | '~'))
    {
        format!("\"{s}\"")
    } else {
        s
    }
}

fn restart_policy(r: Restart) -> &'static str {
    match r {
        Restart::Never => "no",
        Restart::OnFailure => "on-failure",
        Restart::Always => "always",
    }
}

pub fn unit(s: &Service, ctx: &Ctx) -> String {
    let log = s.log_path(&ctx.config);
    let argv: Vec<String> = run_argv(ctx, &s.id).iter().map(|a| quote(a)).collect();
    let mut out = String::new();

    out.push_str("[Unit]\n");
    out.push_str(&format!("Description={}\n", s.title()));
    out.push('\n');

    out.push_str("[Service]\n");
    out.push_str("Type=simple\n");
    out.push_str(&format!("ExecStart={}\n", argv.join(" ")));
    for (k, v) in &s.env {
        out.push_str(&format!("Environment=\"{k}={}\"\n", v.replace('"', "\\\"")));
    }
    if let Some(cwd) = &s.cwd {
        out.push_str(&format!("WorkingDirectory={}\n", cwd.display()));
    }
    out.push_str(&format!("Restart={}\n", restart_policy(s.restart)));
    if s.restart != Restart::Never {
        out.push_str(&format!("RestartSec={}\n", s.restart_secs));
    }
    // append: 需要 systemd >= 240；比 journal 更容易跨平台统一查看
    out.push_str(&format!("StandardOutput=append:{}\n", log.display()));
    out.push_str(&format!("StandardError=append:{}\n", log.display()));
    out.push('\n');

    out.push_str("[Install]\n");
    out.push_str(&format!(
        "WantedBy={}\n",
        match s.scope {
            Scope::User => "default.target",
            Scope::System => "multi-user.target",
        }
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use crate::Config;

    fn build(restart: &str, scope: &str) -> (Service, Ctx) {
        let text = format!(
            r#"
[[services]]
id = "svc"
name = "我的服务"
command = "/opt/bin/run"
args = ["--path", "/data/my dir", "100%"]
env = {{ TOKEN = "abc" }}
cwd = "/srv"
restart = "{restart}"
scope = "{scope}"
"#
        );
        let cfg = Config::parse(&text).unwrap();
        let ctx = Ctx::with_wrapper(
            Path::new("/etc/ushell/services.toml"),
            PathBuf::from("/usr/local/bin/us-svc"),
        );
        (cfg.services[0].clone(), ctx)
    }

    #[test]
    fn unit_path_follows_scope() {
        let (mut s, _) = build("never", "user");
        assert!(unit_path(&s).ends_with(".config/systemd/user/svc.service"));
        s.scope = Scope::System;
        assert_eq!(unit_path(&s), PathBuf::from("/etc/systemd/system/svc.service"));
    }

    #[test]
    fn quotes_arguments_that_systemd_would_split() {
        assert_eq!(quote("/data/my dir"), "\"/data/my dir\"");
        assert_eq!(quote("100%"), "100%%", "% 必须转义，否则被当成说明符");
        assert_eq!(quote("a;b"), "\"a;b\"");
        assert_eq!(quote("$HOME"), "$$HOME", "$ 必须转义，否则被当变量替换");

        // 配置路径带空格时，ExecStart 里必须整体加引号
        let (s, _) = build("never", "user");
        let ctx = Ctx::with_wrapper(
            Path::new("/etc/my cfg/services.toml"),
            PathBuf::from("/usr/local/bin/us-svc"),
        );
        let u = unit(&s, &ctx);
        assert!(
            u.contains("--config \"/etc/my cfg/services.toml\""),
            "含空格的配置路径要加引号:\n{u}"
        );
    }

    #[test]
    fn service_args_stay_in_config_not_in_the_unit() {
        // 设计要点：调度器只调包装器，用户参数由包装器从配置里读
        let (s, ctx) = build("never", "user");
        let u = unit(&s, &ctx);
        assert!(!u.contains("/data/my dir"), "用户参数不应出现在 unit 里:\n{u}");
        assert!(!u.contains("/opt/bin/run"), "目标程序不应出现在 unit 里:\n{u}");
    }

    #[test]
    fn restart_policy_and_interval_only_when_needed() {
        let (never, ctx) = build("never", "user");
        let u = unit(&never, &ctx);
        assert!(u.contains("Restart=no"));
        assert!(!u.contains("RestartSec"), "Never 不该写 RestartSec");

        let (onfail, _) = build("on_failure", "user");
        assert!(unit(&onfail, &ctx).contains("Restart=on-failure"));

        let (always, _) = build("always", "user");
        let u = unit(&always, &ctx);
        assert!(u.contains("Restart=always"));
        assert!(u.contains("RestartSec=10"));
    }

    #[test]
    fn log_goes_to_file_and_wantedby_matches_scope() {
        let (user, ctx) = build("never", "user");
        let u = unit(&user, &ctx);
        assert!(u.contains("StandardOutput=append:/etc/ushell/logs/svc.log"));
        assert!(u.contains("WantedBy=default.target"));

        let (system, _) = build("never", "system");
        let u = unit(&system, &ctx);
        assert!(u.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn execstart_points_at_wrapper() {
        let (s, ctx) = build("never", "user");
        let u = unit(&s, &ctx);
        assert!(u.contains("ExecStart=/usr/local/bin/us-svc run --config /etc/ushell/services.toml --id svc"));
    }
}
