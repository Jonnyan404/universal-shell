//! 受管子进程的启停。每个程序只允许一个存活实例，用 HashMap 跟踪。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Child;
use std::time::SystemTime;

use anyhow::{anyhow, Context};
use log::info;
use rust_i18n::t;

/// 让子进程不弹出控制台窗口（黑窗）。
///
/// 实现统一放在 `crate::hide_console`——lease / insight 里的 tasklist、netstat
/// 也要用同一份，早先各写各的漏了好几处（Windows 上状态轮询会不停闪黑框）。
fn suppress_console(cmd: &mut std::process::Command) {
    crate::hide_console(cmd);
}

/// 正在运行且被壳持有的子进程
#[derive(Default)]
pub struct Runner {
    /// program id -> 子进程句柄
    children: BTreeMap<String, Child>,
    /// program id -> 启动时刻。用来算「运行时长」；进程退出/被停止时同步清掉。
    /// 只覆盖壳托管的程序——系统调度的程序由 launchd/systemd 拉起，壳看不到启动时刻。
    started: BTreeMap<String, SystemTime>,
    /// program id -> 最近一次「自己退出」的退出码。sweep 线程据此写事件时间线，
    /// 让用户看到的是「异常退出 code 1」而不是笼统的「已停止」。
    /// 主动 stop 不算退出，会先清掉这一项。
    exits: BTreeMap<String, Option<i32>>,
}

impl Runner {
    pub fn new() -> Self {
        Self::default()
    }

    /// 启动：is_running 检查是否已存活(有句柄未轮询回收)
    pub fn is_running(&mut self, id: &str) -> bool {
        if let Some(child) = self.children.get_mut(id) {
            match child.try_wait() {
                Ok(Some(st)) => {
                    self.children.remove(id);
                    self.started.remove(id);
                    // 进程没了，泵也没必要再轮询；退出前会把最后几行抽干净
                    crate::logpump::unwatch(id);
                    // 记下退出码：sweep 线程会把「自己退了」写成一条事件
                    self.exits.insert(id.to_string(), st.code());
                    false
                }
                Ok(None) => true,
                Err(_) => true,
            }
        } else {
            false
        }
    }

    /// 清扫已退出的子进程（外部 kill / 自然退出 / 崩溃），返回 id 列表并移除句柄。
    /// F-3(F10)：事件推送的前端，web-server 后台 watcher 定时调用。
    pub fn sweep_exited(&mut self) -> Vec<String> {
        let ids: Vec<String> = self.children.keys().cloned().collect();
        let mut exited = Vec::new();
        for id in ids {
            if !self.is_running(&id) {
                exited.push(id);
            }
        }
        exited
    }


    /// 前台启动(窗口应用用这个)：stdout/stderr 直接落到两个普通文件，
    /// 壳侧的「泵」再把它们合并进 `{id}.log`（stderr 行带 `\x1F` 前缀供前端着色）。
    ///
    /// **为什么不是 `Stdio::piped()`**：管道读端在壳进程里，壳被 SIGKILL 后读端消失，
    /// 子进程下一次 write 就 EPIPE 自杀（详见 `logpump` 模块文档）。
    /// 写普通文件则永远成功，壳死不死都不影响子进程。
    ///
    /// 日志采用「会话追加」语义：不截断历史，逐会话写入一条分隔标记
    /// （与 shell.log 同策略），界面始终能见到上次会话内容、避免空窗。
    pub fn start_async(
        &mut self,
        id: &str,
        bin_path: &PathBuf,
        args: &[String],
        env: &[(String, String)],
        working_dir: &PathBuf,
        log_dir: &PathBuf,
    ) -> anyhow::Result<()> {
        if self.is_running(id) {
            return Err(anyhow!(t!("err.runner.already", id = id)));
        }

        std::fs::create_dir_all(log_dir)?;
        let log_path = crate::logpump::merged_path(log_dir, id);
        // 会话标记：与上次内容接在同一个文件里，泵按偏移继续往后追加
        let prior_len = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
        {
            use std::io::Write as _;
            use time::format_description::well_known::Rfc3339;
            let stamp = time::OffsetDateTime::now_local()
                .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
                .format(&Rfc3339)
                .unwrap_or_else(|_| "?".into());
            let mut marker = String::new();
            if prior_len > 0 {
                marker.push('\n');
            }
            marker.push_str(&t!("log.session_started", time = stamp));
            marker.push('\n');
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                let _ = f.write_all(marker.as_bytes());
                let _ = f.flush();
            }
        }

        // 子进程的 stdout/stderr 直接指向普通文件：壳怎么死都不会 EPIPE
        let (out_file, err_file) = crate::logpump::open_sources(log_dir, id)?;
        // 上一轮已经合并过的内容不能再合一遍：偏移对齐到源文件当前末尾
        crate::logpump::reset_offsets(log_dir, id);

        let mut cmd = std::process::Command::new(bin_path);
        cmd.args(args)
            .envs(env.iter().cloned())
            .current_dir(working_dir)
            .stdout(std::process::Stdio::from(out_file))
            .stderr(std::process::Stdio::from(err_file))
            .stdin(std::process::Stdio::null());
        // Windows「无黑窗」：壳是 GUI 进程，直接 spawn 控制台程序会闪黑窗，
        // 派生前设 CREATE_NO_WINDOW；受管程序的 stdout/stderr 已重定向到文件，
        // 无需可见控制台，不影响其功能。unix 上为 no-op。
        suppress_console(&mut cmd);
        #[cfg(unix)]
        {
            // 子进程忽略 SIGPIPE：壳退出后如果还有残留的管道写端（比如程序自己
            // 又拉了个管道给子进程），写 EPIPE 只会返回错误而不是被信号杀死。
            // 现在 stdout/stderr 已经是普通文件，这条是防御性兜底。
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::signal(libc::SIGPIPE, libc::SIG_IGN);
                    Ok(())
                });
            }
        }

        let child = cmd
            .spawn()
            .with_context(|| t!("err.runner.launch", path = bin_path.display()).to_string())?;
        info!("{}", t!("log.runner.started", id = id, pid = child.id()));

        // 泵：轮询两个源文件，合并进 {id}.log 并广播 log-tick
        crate::logpump::watch(log_dir, id, false);

        self.children.insert(id.to_string(), child);
        self.started.insert(id.to_string(), SystemTime::now());
        // 新一轮运行，上一轮的退出码作废
        self.exits.remove(id);
        Ok(())
    }

    /// 壳持有的子进程 PID（未持有或已退出返回 None）。
    pub fn child_pid(&self, id: &str) -> Option<u32> {
        self.children.get(id).map(|c| c.id())
    }

    /// 已运行秒数。没被壳托管（系统调度 / 未运行）返回 None。
    pub fn uptime_secs(&self, id: &str) -> Option<u64> {
        self.started.get(id).and_then(|t| t.elapsed().ok()).map(|d| d.as_secs())
    }

    /// 取走「最近一次自行退出」的退出码（取一次就清）。sweep 用它区分
    /// 「程序自己退了 / 崩了」和「被用户停掉」，只有前者才写退出事件。
    pub fn take_exit(&mut self, id: &str) -> Option<Option<i32>> {
        self.exits.remove(id)
    }

    /// 停止指定程序。等待几秒优雅退出，超时强杀。
    pub fn stop(&mut self, id: &str) -> anyhow::Result<()> {
        if !self.is_running(id) {
            return Err(anyhow!(t!("err.runner.not_running", id = id)));
        }
        let mut child = self.children.remove(id).unwrap();
        self.started.remove(id);
        // 主动停止不是「退出」，别让 sweep 把它写成异常退出事件
        self.exits.remove(id);
        // 先试 SIGTERM
        child.kill().ok(); // kill() on std Child == SIGKILL on unix
        let wait = child.wait().context(t!("err.runner.wait"))?;
        // 进程已停，泵退出前做最后一次抽取，别把退出前那几行丢掉
        crate::logpump::unwatch(id);
        if !wait.success() {
            info!("{}", t!("log.runner.nonzero", id = id, code = format!("{:?}", wait.code())));
        }
        info!("{}", t!("log.runner.stopped", id = id));
        Ok(())
    }

    /// 退出壳时停止所有
    pub fn stop_all(&mut self) {
        let ids: Vec<String> = self.children.keys().cloned().collect();
        for id in ids {
            let _ = self.stop(&id);
        }
    }
}

/// 生命周期由壳接管：壳对象销毁（正常退出 / 崩溃 / panic 退出）时，
/// 强制终止所有仍持有的子进程，避免残留孤儿在后台继续占用端口。
impl Drop for Runner {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 运行时长：启动后立刻有值，停止后归 None。
    /// 表格「运行时长」列和检查器概览都读它，取错会显示成「—」或一直不涨。
    #[test]
    fn uptime_tracks_start_and_stop() {
        let dir = std::env::temp_dir().join("cc-runner-uptime");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 用系统 sleep 当被测程序，避免依赖测试二进制自身
        let sleep_bin = std::path::PathBuf::from("/bin/sleep");
        if !sleep_bin.exists() {
            return; // 非 unix 环境跳过
        }
        let mut r = Runner::new();
        assert_eq!(r.uptime_secs("sleeper"), None, "没启动过就该是 None");
        r.start_async(
            "sleeper",
            &sleep_bin,
            &["30".to_string()],
            &[],
            &dir,
            &dir,
        )
        .unwrap();
        let up = r.uptime_secs("sleeper").expect("启动后应有运行时长");
        assert!(up < 5, "刚启动的运行时长不该超过 5 秒，实际 {up}");
        r.stop("sleeper").unwrap();
        assert_eq!(r.uptime_secs("sleeper"), None, "停止后应清掉启动时刻");
        let _ = std::fs::remove_dir_all(&dir);
    }

}
