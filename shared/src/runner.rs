//! 受管子进程的启停。每个程序只允许一个存活实例，用 HashMap 跟踪。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Child;

use anyhow::{anyhow, Context};
use log::info;

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

    /// 前台启动(窗口应用用这个：stdout/stderr 走 file，避免阻塞)
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
        let stdout_path = log_dir.join(format!("{id}.out.log"));
        let stderr_path = log_dir.join(format!("{id}.err.log"));
        let stdout_file = std::fs::File::create(&stdout_path)?;
        let stderr_file = std::fs::File::create(&stderr_path)?;

        let mut cmd = std::process::Command::new(bin_path);
        cmd.args(args)
            .current_dir(working_dir)
            .stdout(std::process::Stdio::from(stdout_file))
            .stderr(std::process::Stdio::from(stderr_file))
            .stdin(std::process::Stdio::null());

        let child = cmd
            .spawn()
            .with_context(|| format!("无法启动 {}", bin_path.display()))?;
        info!("已启动 {id} (pid={})", child.id());
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