//! 包装器：被调度器调起的那一层。
//!
//! 为什么不让调度器直接调目标程序？
//! - Windows 计划任务不接管 stdout/stderr，没有这层就永远看不到日志；
//! - 延迟统一在这里做（launchd 没有原生 delay），三平台语义一致；
//! - 退出码、启动时间、PID 都写进同一个日志，排障只需要看一个文件；
//! - 调度器的 SIGTERM 会转发给子进程，服务能优雅退出（否则子进程会被晾成孤儿）；
//! - 日志超过上限时滚动一份 `.1`，长期运行不会把磁盘写满。

use crate::Service;
use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// 单个日志文件上限：超过就滚到 `<name>.1`（只保留一份历史，够排障不占地方）
const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;

fn stamp() -> String {
    time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "?".into())
}


/// 超过上限就把当前日志改名成 `<name>.1`（覆盖旧的那份），下次写入自动新建。
fn rotate_if_needed_with(path: &Path, limit: u64) -> Result<()> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(()); // 还不存在
    };
    if meta.len() <= limit {
        return Ok(());
    }
    let bak = path.with_extension("log.1");
    let _ = std::fs::remove_file(&bak);
    std::fs::rename(path, &bak)
        .with_context(|| format!("滚动日志 {} 失败", path.display()))?;
    Ok(())
}

/// 把调度器发给包装器的终止信号原样转给子进程。
///
/// 装了这个处理器之后，包装器不会立即随 SIGTERM 退出，而是继续 wait 子进程 ——
/// 这正是我们要的：先让服务自己收尾，再由调度器在超时后强杀。
#[cfg(unix)]
fn forward_term_signals(child_pid: u32) {
    use signal_hook::{consts::TERM_SIGNALS, iterator::Signals};
    if let Ok(mut signals) = Signals::new(TERM_SIGNALS) {
        std::thread::spawn(move || {
            for sig in signals.forever() {
                // 安全性：kill 只投递信号，不改内存；pid 由我们自己 spawn
                unsafe {
                    libc::kill(child_pid as i32, sig);
                }
            }
        });
    }
}

/// 拉起到结束的完整流程；返回子进程退出码（拿不到时返回 1）
pub fn run(service: &Service, log_path: &Path) -> Result<i32> {
    if let Some(dir) = log_path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("创建日志目录 {} 失败", dir.display()))?;
    }
    rotate_if_needed_with(log_path, MAX_LOG_BYTES)?;
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .with_context(|| format!("打开日志 {} 失败", log_path.display()))?;

    if service.delay_secs > 0 {
        writeln!(log, "=== {} delay {}s ===", stamp(), service.delay_secs)?;
        let _ = log.flush();
        std::thread::sleep(Duration::from_secs(service.delay_secs));
    }

    let mut cmd = Command::new(&service.command);
    cmd.args(&service.args);
    for (k, v) in &service.env {
        cmd.env(k, v);
    }
    if let Some(cwd) = &service.cwd {
        cmd.current_dir(cwd);
    }
    // 子进程的 stdout/stderr 直接落到同一个日志句柄
    let out = log.try_clone().context("复制日志句柄失败")?;
    let err = log.try_clone().context("复制日志句柄失败")?;
    cmd.stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .stdin(Stdio::null());

    let mut child = cmd
        .spawn()
        .with_context(|| format!("启动 {} 失败", service.command))?;
    let pid = child.id();
    writeln!(log, "=== {} start pid={} ===", stamp(), pid)?;
    let _ = log.flush();
    #[cfg(unix)]
    forward_term_signals(pid);

    let status = child.wait().context("等待子进程失败")?;
    let code = status.code().unwrap_or(1);
    writeln!(log, "=== {} exit code={} ===", stamp(), code)?;
    let _ = log.flush();
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    fn tmp_log(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("us-svc-{}-{}.log", name, std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn service(toml: &str) -> Service {
        Config::parse(toml).unwrap().services.remove(0)
    }

    #[cfg(unix)]
    #[test]
    fn captures_output_and_propagates_exit_code() {
        let log = tmp_log("exit");
        let s = service(
            r#"
[[services]]
id = "t"
command = "/bin/sh"
args = ["-c", "echo hello-from-service; echo oops >&2; exit 3"]
"#,
        );
        let code = run(&s, &log).unwrap();
        assert_eq!(code, 3, "退出码要原样传回调度器");

        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("hello-from-service"), "stdout 要进日志:\n{text}");
        assert!(text.contains("oops"), "stderr 也要进日志:\n{text}");
        assert!(text.contains("exit code=3"), "要记录退出码:\n{text}");
        assert!(text.contains("start pid="), "要记录 pid:\n{text}");
        let _ = std::fs::remove_file(&log);
    }

    #[cfg(unix)]
    #[test]
    fn injects_env_and_working_dir() {
        let log = tmp_log("env");
        let dir = std::env::temp_dir().join(format!("us-svc-cwd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = service(&format!(
            r#"
[[services]]
id = "t"
command = "/bin/sh"
args = ["-c", "echo token=$US_TOKEN; pwd"]
env = {{ US_TOKEN = "s3cr3t" }}
cwd = "{}"
"#,
            dir.display()
        ));
        run(&s, &log).unwrap();
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("token=s3cr3t"), "env 未注入:\n{text}");
        assert!(text.contains(&dir.to_string_lossy().to_string()), "cwd 未生效:\n{text}");
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn reports_missing_command_instead_of_panicking() {
        let log = tmp_log("missing");
        let s = service(
            r#"
[[services]]
id = "t"
command = "/definitely/not/here"
"#,
        );
        let err = run(&s, &log).unwrap_err().to_string();
        assert!(err.contains("启动"), "实际: {err}");
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn rotates_log_past_the_limit_and_keeps_one_backup() {
        let dir = std::env::temp_dir().join(format!("us-svc-rotate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("a.log");
        std::fs::write(&log, "x".repeat(500)).unwrap();

        rotate_if_needed_with(&log, 1024).unwrap();
        assert!(log.exists(), "未超限不该滚动");

        rotate_if_needed_with(&log, 100).unwrap();
        assert!(!log.exists(), "超限后原文件应被移走");
        let bak = dir.join("a.log.1");
        assert_eq!(std::fs::read_to_string(&bak).unwrap().len(), 500);

        // 第二次滚动覆盖旧的 .1，不累积
        std::fs::write(&log, "y".repeat(500)).unwrap();
        rotate_if_needed_with(&log, 100).unwrap();
        assert_eq!(std::fs::read_to_string(&bak).unwrap().len(), 500);
        assert!(std::fs::read_to_string(&bak).unwrap().starts_with('y'));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn repeated_runs_append_rather_than_truncate() {
        let log = tmp_log("append");
        let s = service(
            r#"
[[services]]
id = "t"
command = "/bin/sh"
args = ["-c", "echo run"]
"#,
        );
        run(&s, &log).unwrap();
        run(&s, &log).unwrap();
        let text = std::fs::read_to_string(&log).unwrap();
        assert_eq!(text.matches("=== ").count(), 4, "每次运行写 start+exit 两行标记:\n{text}");
        assert_eq!(text.matches("run\n").count(), 2, "两次运行都应保留:\n{text}");
        let _ = std::fs::remove_file(&log);
    }
}
