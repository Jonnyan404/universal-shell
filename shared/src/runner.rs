//! 受管子进程的启停。每个程序只允许一个存活实例，用 HashMap 跟踪。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Child;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context};
use log::info;

/// 把一路字节流逐行写入共享日志。`is_stderr` 时行首加 \x1F 标记(供前端着色)。
fn copy_stream_lines<R: std::io::BufRead>(
    r: &mut R,
    writer: &Arc<Mutex<std::io::BufWriter<std::fs::File>>>,
    is_stderr: bool,
) {
    use std::io::Write as _;
    let mut buf = String::new();
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
                }
            }
        }
    }
}

/// 正在运行且被壳持有的子进程
pub struct Runner {
    /// program id -> 子进程句柄
    children: BTreeMap<String, Child>,
}

impl Default for Runner {
    fn default() -> Self {
        Self { children: BTreeMap::new() }
    }
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

    /// 按可执行文件路径杀死遗留的孤儿进程（壳重启后子进程句柄已丢失，
    /// 若该程序仍存活在系统上则会占用端口，导致无法再次启动）。
    /// 返回是否杀掉了进程。
    #[cfg(target_os = "macos")]
    pub fn kill_orphan_by_path(&self, bin_path: &PathBuf) -> bool {
        let out = std::process::Command::new("pgrep")
            .arg("-f")
            .arg(bin_path.display().to_string())
            .output();
        let Ok(out) = out else { return false };
        let Ok(text) = String::from_utf8(out.stdout) else {
            return false;
        };
        let mut killed = false;
        for line in text.lines() {
            let pid = line.trim();
            if pid.is_empty() || !pid.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            // 跳过自己……壳进程本身不可能与受管二进制同路径，直接 kill
            let _ = std::process::Command::new("kill")
                .arg(pid)
                .status();
            killed = true;
        }
        killed
    }

    /// 非 macos 兜底：尽力而为（暂无实现），返回 false
    #[cfg(not(target_os = "macos"))]
    pub fn kill_orphan_by_path(&self, _bin_path: &PathBuf) -> bool {
        false
    }

    /// 前台启动(窗口应用用这个)：stdout/stderr 合并写入同一日志文件，
    /// 其中 stderr 行以记录分隔符 `\x1F` 开头，供前端着色区分。
    pub fn start_async(
        &mut self,
        id: &str,
        bin_path: &PathBuf,
        args: &[String],
        working_dir: &PathBuf,
        log_dir: &PathBuf,
    ) -> anyhow::Result<()> {
        if self.is_running(id) {
            return Err(anyhow!("{id} 已在运行"));
        }

        std::fs::create_dir_all(log_dir)?;
        let log_path = log_dir.join(format!("{id}.log"));
        let log_file = std::fs::File::create(&log_path)?;

        let mut cmd = std::process::Command::new(bin_path);
        cmd.args(args)
            .current_dir(working_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("无法启动 {}", bin_path.display()))?;
        info!("已启动 {id} (pid={})", child.id());

        // 后台线程把 stdout/stderr 两路写到同一文件；stderr 行加前缀
        let out = child.stdout.take();
        let err = child.stderr.take();
        let writer = std::sync::Arc::new(std::sync::Mutex::new(std::io::BufWriter::new(log_file)));
        let (out, err) = (out.map(std::io::BufReader::new), err.map(std::io::BufReader::new));
        if let Some(mut out) = out {
            let w = writer.clone();
            std::thread::spawn(move || {
                copy_stream_lines(&mut out, &w, false);
            });
        }
        if let Some(mut err) = err {
            let w = writer.clone();
            std::thread::spawn(move || {
                copy_stream_lines(&mut err, &w, true);
            });
        }

        self.children.insert(id.to_string(), child);
        Ok(())
    }

    /// 停止指定程序。等待几秒优雅退出，超时强杀。
    pub fn stop(&mut self, id: &str) -> anyhow::Result<()> {
        if !self.is_running(id) {
            return Err(anyhow!("{id} 未在运行"));
        }
        let mut child = self.children.remove(id).unwrap();
        // 先试 SIGTERM
        child.kill().ok(); // kill() on std Child == SIGKILL on unix
        let wait = child.wait().context("等待子进程退出失败")?;
        if !wait.success() {
            info!("{id} 非零退出码: {:?}", wait.code());
        }
        info!("已停止 {id}");
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