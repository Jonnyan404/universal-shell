//! Windows：任务计划程序（Task Scheduler）XML。
//!
//! 为什么不用注册表 `Run` 键：它不支持延迟、失败重启、提权，也无法捕获 stdout。
//! 计划任务支持这些，代价是必须用本 crate 的 `run` 包装器来重定向日志。

use crate::{config_dir, run_argv, Ctx, Restart, Scope, Service};
use std::path::PathBuf;

/// 任务路径（`/` 分组，便于在任务计划程序里找到）
pub fn task_name(id: &str) -> String {
    format!("ushell\\{id}")
}

/// XML 落盘位置：放在配置目录下，`apply` 时用 `/XML` 导入
pub fn xml_path(s: &Service, config: &PathBuf) -> PathBuf {
    config_dir(config)
        .join("schtasks")
        .join(format!("{}.xml", s.id))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Windows 命令行拼参：含空白/引号时按 MSVCRT 规则加引号并转义反斜杠
fn quote_arg(a: &str) -> String {
    if !a.is_empty() && !a.chars().any(|c| c.is_whitespace() || c == '"') {
        return a.to_string();
    }
    let mut out = String::from("\"");
    let mut slashes = 0usize;
    for ch in a.chars() {
        match ch {
            '\\' => {
                slashes += 1;
                out.push('\\');
            }
            '"' => {
                for _ in 0..slashes {
                    out.push('\\');
                }
                slashes = 0;
                out.push_str("\\\"");
            }
            c => {
                slashes = 0;
                out.push(c);
            }
        }
    }
    for _ in 0..slashes {
        out.push('\\');
    }
    out.push('"');
    out
}

pub fn xml(s: &Service, ctx: &Ctx) -> String {
    let argv = run_argv(ctx, &s.id);
    let command = &argv[0];
    let arguments = argv[1..].iter().map(|a| quote_arg(a)).collect::<Vec<_>>().join(" ");
    let working_dir = s
        .cwd
        .clone()
        .unwrap_or_else(|| config_dir(&ctx.config));

    let trigger = match s.scope {
        Scope::User => "    <LogonTrigger>\n      <Enabled>true</Enabled>\n    </LogonTrigger>\n",
        Scope::System => "    <BootTrigger>\n      <Enabled>true</Enabled>\n    </BootTrigger>\n",
    };
    let run_level = match s.scope {
        Scope::User => "LeastPrivilege",
        Scope::System => "HighestAvailable",
    };
    let restart_block = if s.restart == Restart::Never {
        String::new()
    } else {
        // 计划任务只在「失败/异常结束」时重启，无法像 systemd 那样对正常退出也重启
        format!(
            "    <RestartOnFailure>\n      <Interval>PT{}S</Interval>\n      <Count>999</Count>\n    </RestartOnFailure>\n",
            s.restart_secs.max(1)
        )
    };

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<Task version=\"1.4\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n");
    out.push_str("  <RegistrationInfo>\n");
    out.push_str(&format!(
        "    <Description>{}</Description>\n",
        esc(&format!("{} (managed by us-svc)", s.title()))
    ));
    out.push_str(&format!("    <URI>{}</URI>\n", esc(&task_name(&s.id))));
    out.push_str("  </RegistrationInfo>\n");

    out.push_str("  <Triggers>\n");
    out.push_str(trigger);
    out.push_str("  </Triggers>\n");

    out.push_str("  <Principals>\n    <Principal id=\"Author\">\n");
    out.push_str(&format!("      <RunLevel>{run_level}</RunLevel>\n"));
    out.push_str("    </Principal>\n  </Principals>\n");

    out.push_str("  <Settings>\n");
    out.push_str("    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\n");
    out.push_str("    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\n");
    out.push_str("    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\n");
    out.push_str("    <AllowHardTerminate>true</AllowHardTerminate>\n");
    out.push_str("    <StartWhenAvailable>true</StartWhenAvailable>\n");
    out.push_str(restart_block.as_str());
    // 常驻服务：不设执行时长上限
    out.push_str("    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\n");
    out.push_str("    <Enabled>true</Enabled>\n");
    out.push_str("  </Settings>\n");

    out.push_str("  <Actions Context=\"Author\">\n    <Exec>\n");
    out.push_str(&format!("      <Command>{}</Command>\n", esc(command)));
    out.push_str(&format!("      <Arguments>{}</Arguments>\n", esc(&arguments)));
    out.push_str(&format!(
        "      <WorkingDirectory>{}</WorkingDirectory>\n",
        esc(&working_dir.to_string_lossy())
    ));
    out.push_str("    </Exec>\n  </Actions>\n");
    out.push_str("</Task>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use crate::Config;

    fn build(scope: &str, restart: &str) -> (Service, Ctx) {
        let text = format!(
            r#"
[[services]]
id = "svc"
name = "后台服务"
command = "C:\\tools\\svc.exe"
args = ["--data", "C:\\Users\\me\\My Data", "a\"b"]
cwd = "C:\\srv"
restart = "{restart}"
scope = "{scope}"
"#
        );
        let cfg = Config::parse(&text).unwrap();
        let ctx = Ctx::with_wrapper(
            Path::new(r"C:\cfg\services.toml"),
            PathBuf::from(r"C:\tools\us-svc.exe"),
        );
        (cfg.services[0].clone(), ctx)
    }

    #[test]
    fn task_name_is_grouped() {
        assert_eq!(task_name("dufs"), "ushell\\dufs");
    }

    #[test]
    fn command_points_at_wrapper_and_quotes_paths_with_spaces() {
        let (s, _) = build("user", "never");
        let ctx = Ctx::with_wrapper(
            Path::new(r"C:\my cfg\services.toml"),
            PathBuf::from(r"C:\tools\us-svc.exe"),
        );
        let x = xml(&s, &ctx);
        assert!(x.contains(r"<Command>C:\tools\us-svc.exe</Command>"), "{x}");
        assert!(
            x.contains(r#"run --config &quot;C:\my cfg\services.toml&quot; --id svc"#),
            "含空格的配置路径要整体加引号（XML 里表现为 &quot;）:\n{x}"
        );
    }

    #[test]
    fn quotes_and_escapes_like_the_windows_crt() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(quote_arg("a b"), "\"a b\"");
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn service_args_stay_in_config_not_in_the_task() {
        // 计划任务只管拉起包装器；目标程序与参数由包装器从 TOML 读取
        let (s, ctx) = build("user", "never");
        let x = xml(&s, &ctx);
        assert!(!x.contains("My Data"), "用户参数不应出现在任务里:\n{x}");
        assert!(
            !x.contains(r"C:\tools\svc.exe"),
            "目标程序不应直接被计划任务执行（应经包装器）:\n{x}"
        );
    }

    #[test]
    fn trigger_and_run_level_follow_scope() {
        let (user, ctx) = build("user", "never");
        let u = xml(&user, &ctx);
        assert!(u.contains("<LogonTrigger>"));
        assert!(u.contains("LeastPrivilege"));

        let (system, _) = build("system", "never");
        let s = xml(&system, &ctx);
        assert!(s.contains("<BootTrigger>"));
        assert!(s.contains("HighestAvailable"));
    }

    #[test]
    fn restart_on_failure_only_when_requested() {
        let (never, ctx) = build("user", "never");
        assert!(!xml(&never, &ctx).contains("RestartOnFailure"));

        let (onfail, _) = build("user", "on_failure");
        let x = xml(&onfail, &ctx);
        assert!(x.contains("<RestartOnFailure>"));
        assert!(x.contains("<Interval>PT10S</Interval>"));
    }

    #[test]
    fn no_execution_time_limit_for_resident_service() {
        let (s, ctx) = build("user", "never");
        assert!(xml(&s, &ctx).contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
    }

    #[test]
    fn xml_path_sits_next_to_config() {
        let (s, _) = build("user", "never");
        let p = xml_path(&s, &PathBuf::from("/cfg/services.toml"));
        assert_eq!(p, PathBuf::from("/cfg/schtasks/svc.xml"));
    }
}
