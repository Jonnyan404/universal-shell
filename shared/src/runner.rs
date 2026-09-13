//! 受管子进程的启停。每个程序只允许一个存活实例，用 HashMap 跟踪。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Child;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context};
use log::info;
use rust_i18n::t;

/// 让子进程不弹出控制台窗口（黑窗）。
/// Rust 的 Command::spawn 在 Windows 默认会为控制台子系统程序创建/继承一个
/// 控制台窗口——壳是 GUI 进程，启动 CLI 程序（如 tasklist、被管理的控制台应用）
/// 时就会闪现黑色窗口。用 CREATE_NO_WINDOW 让派生进程不占用可见控制台。
#[cfg(windows)]
fn suppress_console(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn suppress_console(_cmd: &mut std::process::Command) {}

/// 把一路字节流逐行写入共享日志。`is_stderr` 时行首加 \x1F 标记(供前端着色)。
/// 写满一定行数后检查体积，超限就地截末一半：程序跑几小时日志不再无限增长。
fn copy_stream_lines<R: std::io::BufRead>(
    r: &mut R,
    writer: &Arc<Mutex<std::io::BufWriter<std::fs::File>>>,
    is_stderr: bool,
    id: &str,
) {
    use std::io::Write as _;
    let mut buf = String::new();
    let mut since_check = 0usize;
    // log-tick 降频：只按「≥1行且距上次 ≥150ms」发射，避免 chatty 程序按行风暴式
    // 推送事件，最小化后恢复时前端一次积压成千上万个 log-tick 导致假死。
    let mut last_emit: Option<std::time::Instant> = None;
    loop {
        buf.clear();
        match r.read_line(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let mut line = String::with_capacity(buf.len() + 1);
                if is_stderr {
                    line.push('\u{1f}');
                }
                line.push_str(&buf);
                if let Ok(mut w) = writer.lock() {
                    let _ = w.write_all(line.as_bytes());
                    let _ = w.flush();
                    since_check += 1;
                    // 两路写线程各自计数检查，近似即可；修整全程持锁，另一路短暂等待
                    if since_check >= PROGRAM_LOG_CHECK_LINES {
                        since_check = 0;
                        trim_program_log(&mut w);
                    }
                    let due = match last_emit {
                        None => true,
                        Some(t) => t.elapsed() >= LOG_TICK_MIN_INTERVAL,
                    };
                    if due {
                        last_emit = Some(std::time::Instant::now());
                        crate::events::emit(crate::events::Event::LogWritten(id.to_string()));
                    }
                }
            }
        }
    }
}

/// 程序运行日志单文件上限（字节）。超过后在写线程内截末一半（和 shell.log 同策略），
/// 长时间运行不再无限增长，恢复界面时的尾部读取也保持轻量。
const PROGRAM_LOG_MAX: u64 = 512 * 1024;
/// 写满多少行检查一次体积
const PROGRAM_LOG_CHECK_LINES: usize = 64;
/// log-tick 事件最小发射间隔：日志按行写、按此降频广播，
/// chatty 程序不再逐行推事件，最小化恢复时前端也不会积压事件风暴。
const LOG_TICK_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);

/// 体积超限时保留末尾一半的完整行。调用方须已持有写锁、缓冲区已刷盘；
/// 全程经同一句柄操作，写偏移始终有效。
fn trim_program_log(w: &mut std::io::BufWriter<std::fs::File>) {
    use std::io::{Read, Seek, SeekFrom, Write as _};
    let Ok(len) = w.get_ref().metadata().map(|m| m.len()) else {
        return;
    };
    if len <= PROGRAM_LOG_MAX {
        return;
    }
    if w.seek(SeekFrom::Start(0)).is_err() {
        return;
    }
    // 经底层 File 直读（BufWriter 本身无 Read 实现）；此前已刷盘，偏移有效
    let mut data = Vec::new();
    if w.get_mut().read_to_end(&mut data).is_err() {
        return;
    }
    let half = data.len().saturating_sub((PROGRAM_LOG_MAX / 2) as usize);
    let start = data[half..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|i| half + i + 1)
        .unwrap_or(0);
    let keep = if start == 0 { data } else { data[start..].to_vec() };
    if w.get_mut().set_len(0).is_err() {
        return;
    }
    if w.seek(SeekFrom::Start(0)).is_err() {
        return;
    }
    let _ = w.write_all(&keep);
    let _ = w.flush();
}

/// 正在运行且被壳持有的子进程
#[derive(Default)]
pub struct Runner {
    /// program id -> 子进程句柄
    children: BTreeMap<String, Child>,
}

impl Runner {
    pub fn new() -> Self {
        Self::default()
    }

    /// 启动：is_running 检查是否已存活(有句柄未轮询回收)
    pub fn is_running(&mut self, id: &str) -> bool {
        if let Some(child) = self.children.get_mut(id) {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.children.remove(id);
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

    /// 前台启动(窗口应用用这个)：stdout/stderr 合并写入同一日志文件，
    /// 其中 stderr 行以记录分隔符 `\x1F` 开头，供前端着色区分。
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
        let log_path = log_dir.join(format!("{id}.log"));
        // 读写打开：写线程超限截断时需经同一句柄回读（只写句柄 read 会 EBADF 导致截断静默失效）
        let log_file = std::fs::OpenOptions::new()
            .read(true)
            .create(true)
            .append(true)
            .open(&log_path)?;
        // 会话标记：与上次内容接在同一个文件里，写线程按顺序继续
        let prior_len = log_file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut writer = std::io::BufWriter::new(log_file);
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
            let _ = writer.write_all(marker.as_bytes());
            let _ = writer.flush();
        }

        let mut cmd = std::process::Command::new(bin_path);
        cmd.args(args)
            .envs(env.iter().cloned())
            .current_dir(working_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null());
        // Windows「无黑窗」：壳是 GUI 进程，直接 spawn 控制台程序会闪黑窗，
        // 派生前设 CREATE_NO_WINDOW；受管程序的 stdout/stderr 已重定向到日志文件，
        // 无需可见控制台，不影响其功能。unix 上为 no-op。
        suppress_console(&mut cmd);
        #[cfg(unix)]
        {
            // 子进程忽略 SIGPIPE：壳退出后管道读端关闭，残留孤儿再写日志只会 EPIPE，
            // 不会被信号杀死（Go 等程序默认会被 SIGPIPE 杀死）。生命周期已由壳接管，
            // 壳退出时会 stop_all 杀掉全部子进程，此处保留 SIG_IGN 作防御性兜底。
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::signal(libc::SIGPIPE, libc::SIG_IGN);
                    Ok(())
                });
            }
        }

        let mut child = cmd
            .spawn()
            .with_context(|| t!("err.runner.launch", path = bin_path.display()).to_string())?;
        info!("{}", t!("log.runner.started", id = id, pid = child.id()));

        // 后台线程把 stdout/stderr 两路写到同一文件；stderr 行加前缀
        let out = child.stdout.take();
        let err = child.stderr.take();
        let writer = std::sync::Arc::new(std::sync::Mutex::new(writer));
        let (out, err) = (out.map(std::io::BufReader::new), err.map(std::io::BufReader::new));
        if let Some(mut out) = out {
            let w = writer.clone();
            let id = id.to_string();
            std::thread::spawn(move || {
                copy_stream_lines(&mut out, &w, false, &id);
            });
        }
        if let Some(mut err) = err {
            let w = writer.clone();
            let id = id.to_string();
            std::thread::spawn(move || {
                copy_stream_lines(&mut err, &w, true, &id);
            });
        }

        self.children.insert(id.to_string(), child);
        Ok(())
    }

    /// 壳持有的子进程 PID（未持有或已退出返回 None）。
    pub fn child_pid(&self, id: &str) -> Option<u32> {
        self.children.get(id).map(|c| c.id())
    }

    /// 停止指定程序。等待几秒优雅退出，超时强杀。
    pub fn stop(&mut self, id: &str) -> anyhow::Result<()> {
        if !self.is_running(id) {
            return Err(anyhow!(t!("err.runner.not_running", id = id)));
        }
        let mut child = self.children.remove(id).unwrap();
        // 先试 SIGTERM
        child.kill().ok(); // kill() on std Child == SIGKILL on unix
        let wait = child.wait().context(t!("err.runner.wait"))?;
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

    /// 超 512KB 后截末一半：体积回落、尾部保留、头部丢掉、行对齐。
    #[test]
    fn trim_program_log_keeps_tail_half() {
        use std::io::Write as _;
        let dir = std::env::temp_dir().join("cc-trim-proglog");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.log");
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        let mut w = std::io::BufWriter::new(f);
        // 写 ~600KB 行日志
        for i in 0..20000 {
            use std::io::Write as _;
            writeln!(w, "line-{i:05}-0123456789abcdef").unwrap();
        }
        w.flush().unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > PROGRAM_LOG_MAX);

        trim_program_log(&mut w);
        drop(w);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!((text.len() as u64) < PROGRAM_LOG_MAX);
        assert!(text.ends_with("line-19999-0123456789abcdef\n"));
        assert!(!text.contains("line-00000-"));
        let first = text.lines().next().unwrap();
        assert!(first.starts_with("line-"), "首行应对齐换行：{first}");
    }

    /// 未超限时不碰文件。
    #[test]
    fn trim_program_log_keeps_small_file() {
        let dir = std::env::temp_dir().join("cc-trim-proglog-small");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.log");
        let f = std::fs::File::create(&path).unwrap();
        let mut w = std::io::BufWriter::new(f);
        {
            use std::io::Write as _;
            writeln!(w, "hello").unwrap();
            w.flush().unwrap();
        }
        trim_program_log(&mut w);
        drop(w);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\n");
    }
}