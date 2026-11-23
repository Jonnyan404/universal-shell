//! 端到端验证：调度器发给**包装器**的 SIGTERM 会转发给真正的子进程。
//!
//! 这条不测的话很容易漏：systemd 靠 cgroup 杀整组，但 launchd 的 bootout 只保证
//! 结束作业主进程，子进程可能被晾成孤儿——所以包装器必须自己转发。

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 测试失败也要把包装器收掉，否则会留下孤儿进程
struct KillOnDrop(std::process::Child);

impl KillOnDrop {
    fn id(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for(path: &Path, needle: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(text) = std::fs::read_to_string(path) {
            if text.contains(needle) {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("us-svc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn forwards_sigterm_to_child() {
    let dir = temp_dir("signal");
    let cfg = dir.join("services.toml");
    std::fs::write(
        &cfg,
        r#"
[[services]]
id = "t"
command = "/bin/sh"
args = ["-c", "trap 'echo child-got-term; exit 0' TERM; echo child-started; while :; do sleep 0.1; done"]
"#,
    )
    .unwrap();

    let wrapper = KillOnDrop(
        std::process::Command::new(env!("CARGO_BIN_EXE_us-svc"))
            .args(["run", "--config", cfg.to_str().unwrap(), "--id", "t"])
            .spawn()
            .expect("启动包装器"),
    );

    let log = dir.join("logs/t.log");
    assert!(
        wait_for(&log, "child-started", Duration::from_secs(5)),
        "子进程没起来：{:?}",
        std::fs::read_to_string(&log)
    );

    // 模拟 launchctl bootout / systemctl stop：信号是发给包装器的
    let ok = std::process::Command::new("kill")
        .args(["-TERM", &wrapper.id().to_string()])
        .status()
        .unwrap();
    assert!(ok.success(), "kill 失败");

    assert!(
        wait_for(&log, "child-got-term", Duration::from_secs(5)),
        "SIGTERM 没有转发给子进程：{:?}",
        std::fs::read_to_string(&log)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exits_with_child_code_after_signal_round_trip() {
    let dir = temp_dir("exitcode");
    let cfg = dir.join("services.toml");
    // 收到 TERM 后以 0 退出：包装器应跟着以 0 结束（不再是被信号杀掉）
    std::fs::write(
        &cfg,
        r#"
[[services]]
id = "t"
command = "/bin/sh"
args = ["-c", "trap 'exit 0' TERM; while :; do sleep 0.1; done"]
"#,
    )
    .unwrap();

    let mut wrapper = KillOnDrop(
        std::process::Command::new(env!("CARGO_BIN_EXE_us-svc"))
            .args(["run", "--config", cfg.to_str().unwrap(), "--id", "t"])
            .spawn()
            .expect("启动包装器"),
    );
    let log = dir.join("logs/t.log");
    assert!(wait_for(&log, "start pid=", Duration::from_secs(5)), "未启动");

    let _ = std::process::Command::new("kill")
        .args(["-TERM", &wrapper.id().to_string()])
        .status();

    // 30 秒内应正常结束（子进程退出 0）
    let start = Instant::now();
    loop {
        match wrapper.0.try_wait() {
            Ok(Some(status)) => {
                assert_eq!(status.code(), Some(0), "包装器应透传子进程的 0");
                break;
            }
            Ok(None) if start.elapsed() < Duration::from_secs(10) => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => panic!("包装器没有在收到 TERM 后退出"),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
