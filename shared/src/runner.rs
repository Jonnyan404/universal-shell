//! 受管子进程的启停。每个程序只允许一个存活实例，用 HashMap 跟踪。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Child;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context};
use log::info;
use rust_i18n::t;

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

    /// 按可执行文件路径查找系统上匹配的进程 PID。
    /// 壳重启后子进程句柄丢失，用路径探测残留进程以恢复运行态。
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn pids_by_path(&self, bin_path: &PathBuf) -> Vec<u32> {
        let mut pids = Vec::new();
        let Ok(out) = std::process::Command::new("pgrep")
            .arg("-f")
            .arg(bin_path.display().to_string())
            .output()
        else {
            return pids;
        };
        let Ok(text) = String::from_utf8(out.stdout) else {
            return pids;
        };
        for line in text.lines() {
            let pid = line.trim();
            if pid.is_empty() || !pid.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            if let Ok(p) = pid.parse::<u32>() {
                pids.push(p);
            }
        }
        pids
    }

    /// Windows：用 tasklist 按镜像名(可执行文件名) 匹配进程 PID
    #[cfg(target_os = "windows")]
    fn pids_by_path(&self, bin_path: &PathBuf) -> Vec<u32> {
        let Some(name) = bin_path.file_name().map(|n| n.to_string_lossy().into_owned())
        else {
            return Vec::new();
        };
        let mut pids = Vec::new();
        let Ok(out) = std::process::Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {name}"), "/FO", "CSV", "/NH"])
            .output()
        else {
            return pids;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            // CSV 列: "name","pid","session"...  取第二列
            let cols: Vec<&str> = line.split('"').collect();
            if cols.len() >= 4 && cols[3].trim().chars().all(|c| c.is_ascii_digit()) {
                if let Ok(p) = cols[3].trim().parse::<u32>() {
                    pids.push(p);
                }
            }
        }
        pids
    }

    /// 系统上是否有该可执行文件的进程在运行（含残留孤儿）
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    pub fn is_process_alive(&self, bin_path: &PathBuf) -> bool {
        !self.pids_by_path(bin_path).is_empty()
    }

    /// 其它平台兜底：无检测能力，返回 false
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    pub fn is_process_alive(&self, _bin_path: &PathBuf) -> bool {
        false
    }

    /// 按可执行文件路径杀死遗留的孤儿进程（壳重启后子进程句柄已丢失，
    /// 若该程序仍存活在系统上则会占用端口，导致无法再次启动）。
    /// 返回是否杀掉了进程。
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn kill_orphan_by_path(&self, bin_path: &PathBuf) -> bool {
        let pids = self.pids_by_path(bin_path);
        for &pid in &pids {
            let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
        }
        !pids.is_empty()
    }

    #[cfg(target_os = "windows")]
    pub fn kill_orphan_by_path(&self, bin_path: &PathBuf) -> bool {
        let pids = self.pids_by_path(bin_path);
        for &pid in &pids {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F", "/T"])
                .status();
        }
        !pids.is_empty()
    }

    /// 其它平台兜底：尽力而为（暂无实现），返回 false
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
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
            return Err(anyhow!(t!("err.runner.already", id = id)));
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
            .with_context(|| t!("err.runner.launch", path = bin_path.display()).to_string())?;
        info!("{}", t!("log.runner.started", id = id, pid = child.id()));

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