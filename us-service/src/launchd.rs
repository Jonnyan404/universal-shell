//! macOS：launchd LaunchAgent / LaunchDaemon。

use crate::{run_argv, Ctx, Restart, Scope, Service};
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// 单元文件落点：用户级 `~/Library/LaunchAgents`，系统级 `/Library/LaunchDaemons`
pub fn unit_path(s: &Service) -> PathBuf {
    let dir = match s.scope {
        Scope::User => home().join("Library/LaunchAgents"),
        Scope::System => PathBuf::from("/Library/LaunchDaemons"),
    };
    dir.join(format!("{}.plist", s.label()))
}

/// plist 里只允许这些字符原样出现
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// KeepAlive 只有三种形态：false / true / { SuccessfulExit = false }
fn keep_alive(restart: Restart) -> &'static str {
    match restart {
        Restart::Never => "<false/>",
        // 退出码非 0 才拉起：恰好对应 systemd 的 on-failure
        Restart::OnFailure => "<dict><key>SuccessfulExit</key><false/></dict>",
        Restart::Always => "<true/>",
    }
}

pub fn plist(s: &Service, ctx: &Ctx) -> String {
    let log = s.log_path(&ctx.config);
    let mut out = String::new();
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    out.push('\n');
    out.push_str(
        r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">"#,
    );
    out.push('\n');
    out.push_str("<plist version=\"1.0\">\n<dict>\n");

    line(&mut out, "Label", &esc(&s.label()));
    out.push_str("  <key>ProgramArguments</key>\n  <array>\n");
    for a in run_argv(ctx, &s.id) {
        out.push_str(&format!("    <string>{}</string>\n", esc(&a)));
    }
    out.push_str("  </array>\n");

    if !s.env.is_empty() {
        out.push_str("  <key>EnvironmentVariables</key>\n  <dict>\n");
        for (k, v) in &s.env {
            out.push_str(&format!(
                "    <key>{}</key>\n    <string>{}</string>\n",
                esc(k),
                esc(v)
            ));
        }
        out.push_str("  </dict>\n");
    }

    if let Some(cwd) = &s.cwd {
        line(&mut out, "WorkingDirectory", &esc(&cwd.to_string_lossy()));
    }

    // 登录/开机即拉起；延迟在包装器里做（launchd 没有原生 delay）
    out.push_str("  <key>RunAtLoad</key>\n  <true/>\n");
    out.push_str(&format!(
        "  <key>KeepAlive</key>\n  {}\n",
        keep_alive(s.restart)
    ));
    out.push_str(&format!(
        "  <key>ThrottleInterval</key>\n  <integer>{}</integer>\n",
        s.restart_secs
    ));
    // 非交互后台进程：不抢 CPU/IO 优先级
    out.push_str("  <key>ProcessType</key>\n  <string>Background</string>\n");
    line(&mut out, "StandardOutPath", &esc(&log.to_string_lossy()));
    line(&mut out, "StandardErrorPath", &esc(&log.to_string_lossy()));

    out.push_str("</dict>\n</plist>\n");
    out
}

fn line(out: &mut String, key: &str, value: &str) {
    out.push_str(&format!("  <key>{key}</key>\n  <string>{value}</string>\n"));
}

/// 供 CLI 提示用
pub fn log_hint(s: &Service, config: &Path) -> PathBuf {
    s.log_path(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    fn svc(restart: &str) -> (Service, Ctx) {
        let text = format!(
            r#"
[[services]]
id = "dufs"
name = "dufs 服务"
command = "/usr/local/bin/dufs"
args = ["-p", "5000", "/data & more"]
env = {{ RUST_LOG = "info" }}
cwd = "/srv"
restart = "{restart}"
"#
        );
        let cfg = Config::parse(&text).unwrap();
        let ctx = Ctx::with_wrapper(
            Path::new("/cfg/services.toml"),
            PathBuf::from("/usr/local/bin/us-svc"),
        );
        (cfg.services[0].clone(), ctx)
    }

    #[test]
    fn label_and_path_follow_scope() {
        let (mut s, _) = svc("never");
        s.scope = Scope::User;
        let p = unit_path(&s);
        assert!(p.ends_with("Library/LaunchAgents/com.ushell.dufs.plist"), "{p:?}");
        s.scope = Scope::System;
        assert_eq!(unit_path(&s), PathBuf::from("/Library/LaunchDaemons/com.ushell.dufs.plist"));
    }

    #[test]
    fn plist_contains_wrapper_invocation_and_log() {
        let (s, ctx) = svc("never");
        let p = plist(&s, &ctx);
        assert!(p.contains("<string>com.ushell.dufs</string>"));
        assert!(p.contains("/usr/local/bin/us-svc"));
        assert!(p.contains("<string>run</string>"));
        assert!(p.contains("<string>--id</string>"));
        let log = s.log_path(&ctx.config);
        assert!(
            p.contains(&esc(&log.to_string_lossy())),
            "plist 里应出现推导出的日志路径: {log:?}"
        );
        assert!(p.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(p.contains("<key>WorkingDirectory</key>\n  <string>/srv</string>"));
        assert!(p.contains("<key>RUST_LOG</key>"));
    }

    #[test]
    fn keep_alive_maps_each_restart_policy() {
        for (policy, expect) in [
            ("never", "<key>KeepAlive</key>\n  <false/>"),
            (
                "on_failure",
                "<key>KeepAlive</key>\n  <dict><key>SuccessfulExit</key><false/></dict>",
            ),
            ("always", "<key>KeepAlive</key>\n  <true/>"),
        ] {
            let (s, ctx) = svc(policy);
            assert!(plist(&s, &ctx).contains(expect), "policy={policy}");
        }
    }

    #[test]
    fn escapes_xml_special_chars_in_paths() {
        // 配置路径里出现 & 时必须转义，否则 plist 直接解析失败
        let (s, _) = svc("never");
        let ctx = Ctx::with_wrapper(
            Path::new("/cfg/a&b/services.toml"),
            PathBuf::from("/usr/local/bin/us-svc"),
        );
        let p = plist(&s, &ctx);
        assert!(p.contains("a&amp;b"), "原始 & 必须转义:\n{p}");
        assert!(!p.contains("a&b"), "未转义的 & 不应出现:\n{p}");
    }

    #[test]
    fn service_args_stay_in_config_not_in_the_plist() {
        // 设计要点：plist 只描述「谁来拉起」，真正的命令与参数在 services.toml
        let (s, ctx) = svc("never");
        let p = plist(&s, &ctx);
        assert!(!p.contains("/data &amp; more"), "用户参数不应出现在 plist 里:\n{p}");
        assert!(!p.contains("/usr/local/bin/dufs"), "目标程序不应出现在 plist 里:\n{p}");
    }
}
