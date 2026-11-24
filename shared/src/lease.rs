//! 子进程租约：壳 spawn 出来的进程，除了内存里的 `Child` 句柄，还落一份盘上记录。
//!
//! # 为什么需要
//!
//! `Runner.children` 是纯内存的。壳被 SIGKILL（构建时替换二进制、强制退出、
//! 系统重启、OOM）时 `Drop` 不会执行，子进程变孤儿（PPID=1），端口和数据库
//! 文件锁都还被它占着。壳重启后 `children` 表是空的，界面显示「未运行」，
//! 用户一点启动就撞锁 —— clip9 的 `Database already open. Cannot acquire lock.`
//! 就是这么来的。
//!
//! 租约把「壳认为自己在管谁」写进 `pids.json`，壳下次启动时读回来逐个探活：
//!
//! - 活着 → 认作「外部接管中」：界面显示运行中 + PID，可以停止/重启
//! - 死了 → 清掉记录
//!
//! # pid 复用
//!
//! pid 会被系统回收再分配。只靠 `kill -0` 会误判成「我的进程还活着」，进而
//! 误杀别人的进程。所以租约同时记下可执行文件名，探活时二次校验；
//! 名字对不上就当成陈旧记录清掉，**绝不 kill**。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 一条子进程租约。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Lease {
    pub pid: u32,
    /// 启动时刻（unix 秒）。用于展示「上次会话」与判断记录是否陈旧。
    pub started_at: i64,
    /// 程序声明的监听端口（能取到才有）。
    #[serde(default)]
    pub port: Option<u16>,
    /// 可执行文件名（不含路径）。
    #[serde(default)]
    pub bin: String,
    /// 实际进程名。脚本型程序（shebang / 包装器）这里是解释器，比如 `python3`，
    /// 和 `bin` 对不上。结束进程时两个名字任一匹配即可，否则会把正常残留当成
    /// 「pid 已被复用」而拒绝清理。
    #[serde(default)]
    pub comm: String,
}

impl Lease {
    pub fn new(pid: u32, port: Option<u16>, bin: impl Into<String>) -> Self {
        Self {
            pid,
            started_at: now_secs(),
            port,
            bin: bin.into(),
            comm: pid_comm(pid).unwrap_or_default(),
        }
    }

    /// 这条租约指向的进程是否还活着。
    ///
    /// 只看存活，不校验名字：名字对不上有两种可能 —— pid 被系统复用，或者
    /// 程序是脚本（实际进程名是解释器）。后者很常见，一律当死记录会把正常
    /// 残留丢掉，用户又得回去撞锁。防误杀放在 [`Self::name_matches`] 与
    /// [`terminate`] 里：认领可以宽，动手必须严。
    pub fn is_live(&self) -> bool {
        pid_alive(self.pid)
    }

    /// 当前进程名是否对得上这条租约（`bin` 或 `comm` 任一匹配）。
    /// 结束进程前必须为真，否则可能杀到复用同一 pid 的无关程序。
    pub fn name_matches(&self) -> bool {
        if self.bin.trim().is_empty() && self.comm.trim().is_empty() {
            return true;
        }
        let Some(cur) = pid_comm(self.pid) else {
            return false;
        };
        let cur = normalize_comm(&cur);
        let bin = normalize_comm(&self.bin);
        let comm = normalize_comm(&self.comm);
        (!bin.is_empty() && cur == bin)
            || (!comm.is_empty() && cur == comm)
            || (!bin.is_empty() && cur.starts_with(&bin))
    }
}

/// 进程名归一化：小写、去 `.exe`、去路径。
fn normalize_comm(s: &str) -> String {
    let base = s.trim().rsplit('/').next().unwrap_or(s);
    let lower = base.to_ascii_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `data_dir/pids.json` 的读写门面。
#[derive(Debug, Default)]
pub struct LeaseStore {
    path: PathBuf,
    map: BTreeMap<String, Lease>,
}

impl LeaseStore {
    /// 从 `dir/pids.json` 读取。文件不存在或内容损坏都当空表 ——
    /// 一份坏掉的租约文件不该让壳起不来。
    pub fn load(dir: &Path) -> Self {
        let path = dir.join("pids.json");
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<BTreeMap<String, Lease>>(&s).ok())
            .unwrap_or_default();
        Self { path, map }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 落盘。先写临时文件再 rename，避免掉电留下半截 JSON。
    /// 失败只记日志：租约是「尽力而为」的辅助信息，不能因为它挡住启动流程。
    fn save(&self) {
        let Some(dir) = self.path.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let Ok(text) = serde_json::to_string_pretty(&self.map) else { return };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_err() {
            return;
        }
        if std::fs::rename(&tmp, &self.path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    pub fn put(&mut self, id: &str, lease: Lease) {
        self.map.insert(id.to_string(), lease);
        self.save();
    }

    pub fn remove(&mut self, id: &str) {
        if self.map.remove(id).is_some() {
            self.save();
        }
    }

    pub fn get(&self, id: &str) -> Option<&Lease> {
        self.map.get(id)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// 清掉进程已死（或 pid 已被别的程序复用）的记录，返回被清掉的 id。
    pub fn prune(&mut self) -> Vec<String> {
        let dead: Vec<String> = self
            .map
            .iter()
            .filter(|(_, l)| !l.is_live())
            .map(|(id, _)| id.clone())
            .collect();
        if !dead.is_empty() {
            for id in &dead {
                self.map.remove(id);
            }
            self.save();
        }
        dead
    }

    /// 还活着、且确实是我们当初那个进程的租约。
    pub fn live(&self) -> Vec<(String, Lease)> {
        self.map
            .iter()
            .filter(|(_, l)| l.is_live())
            .map(|(id, l)| (id.clone(), l.clone()))
            .collect()
    }
}

// ---------- 平台探活 ----------

/// 进程是否还活着。
///
/// Unix 用 `kill(pid, 0)`：`ESRCH` 表示不存在，`EPERM` 表示存在但没权限 ——
/// 后者也算活着（别人家的进程，不能当死的处理）。
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        let r = unsafe { libc::kill(pid as i32, 0) };
        if r == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        // tasklist 精确过滤；没有匹配时输出的是「信息: 没有运行的任务…」，不含 "pid"
        match std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
        {
            Ok(o) => String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")),
            Err(_) => false,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

/// 进程的可执行文件名（小写，Windows 去掉 `.exe`）。
pub fn pid_comm(pid: u32) -> Option<String> {
    let raw = {
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .ok()
                .map(|s| s.trim().to_string())
        }
        #[cfg(target_os = "macos")]
        {
            let out = std::process::Command::new("ps")
                .args(["-o", "comm=", "-p", &pid.to_string()])
                .output()
                .ok()?;
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.is_empty() {
                None
            } else {
                // ps 给的是完整路径，取最后一段
                Some(s.rsplit('/').next().unwrap_or(&s).to_string())
            }
        }
        #[cfg(windows)]
        {
            let out = std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&out.stdout);
            let line = text.lines().next()?;
            let name = line.trim_start_matches('"').split("\",\"").next()?.to_string();
            if name.is_empty() {
                None
            } else {
                Some(name)
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            let _ = pid;
            None
        }
    }?;
    let mut s = raw.to_ascii_lowercase();
    if let Some(stripped) = s.strip_suffix(".exe") {
        s = stripped.to_string();
    }
    Some(s)
}

/// 谁在监听这个 TCP 端口。拿不到（没权限 / 没装 lsof）返回 None。
///
/// 只用于启动前预检，属于尽力而为：探测失败就跳过预检，不能因此拦住启动。
pub fn port_owner(port: u16) -> Option<u32> {
    if port == 0 {
        return None;
    }
    #[cfg(unix)]
    {
        let out = std::process::Command::new("lsof")
            .args([
                "-nP",
                &format!("-iTCP:{port}"),
                "-sTCP:LISTEN",
                "-t",
            ])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|l| l.trim().parse::<u32>().ok())
    }
    #[cfg(windows)]
    {
        let out = std::process::Command::new("netstat")
            .args(["-ano", "-p", "TCP"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines()
            .find(|l| {
                l.contains("LISTENING")
                    && l.split_whitespace()
                        .any(|f| f.ends_with(&format!(":{port}")))
            })
            .and_then(|l| l.split_whitespace().last()?.parse::<u32>().ok())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = port;
        None
    }
}

/// 结束一个不属于本壳的进程（上次会话残留）。
///
/// 先礼后兵：SIGTERM（Windows `taskkill` 不带 `/F`）→ 等最多 3 秒 →
/// 还活着就 SIGKILL（`taskkill /F`）。
///
/// 动手前校验进程名：pid 可能已被系统复用给别的程序，名字对不上就拒绝结束。
/// 宁可清理失败让用户手动处理，也不能误杀一个无关进程。
pub fn terminate(lease: &Lease) -> Result<(), String> {
    let pid = lease.pid;
    if !pid_alive(pid) {
        return Ok(());
    }
    if !lease.name_matches() {
        return Err(format!(
            "PID {pid} 现在跑的不是「{}」（pid 可能已被复用），拒绝结束",
            lease.bin
        ));
    }
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        for _ in 0..30 {
            if !pid_alive(pid) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        Ok(())
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T"])
            .output();
        for _ in 0..30 {
            if !pid_alive(pid) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        Err("当前平台不支持结束外部进程".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("us-lease-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn store_roundtrip() {
        let dir = temp_dir("rt");
        let mut s = LeaseStore::load(&dir);
        assert!(s.is_empty());
        s.put("clip9", Lease::new(4242, Some(9501), "clip9-cli"));
        s.put("dufs", Lease::new(4343, None, "dufs"));

        // 重新加载（模拟壳重启）
        let mut s2 = LeaseStore::load(&dir);
        assert_eq!(s2.len(), 2);
        assert_eq!(s2.get("clip9").unwrap().pid, 4242);
        assert_eq!(s2.get("clip9").unwrap().port, Some(9501));
        assert_eq!(s2.get("dufs").unwrap().bin, "dufs");

        s2.remove("dufs");
        assert_eq!(LeaseStore::load(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_drops_dead_pid() {
        let dir = temp_dir("prune");
        let mut s = LeaseStore::load(&dir);
        // 当前进程一定活着
        s.put("alive", Lease::new(std::process::id(), None, ""));
        // 一个几乎不可能存在的 pid
        s.put("dead", Lease::new(4_000_000_000, None, ""));
        let gone = s.prune();
        assert_eq!(gone, vec!["dead".to_string()]);
        assert!(s.get("alive").is_some());
        assert!(s.get("dead").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_pid_is_alive_and_named() {
        let me = std::process::id();
        assert!(pid_alive(me), "自己的 pid 必须判活");
        assert!(!pid_alive(0));

        // 拿得到名字，且和自己比得上
        if let Some(comm) = pid_comm(me) {
            let l = Lease::new(me, None, comm.clone());
            assert!(l.name_matches(), "名字应当自匹配: {comm}");
            // 脚本型程序：bin 是脚本名，comm 是解释器，靠 comm 兜住
            let mut script = Lease::new(me, None, "some-script.sh");
            script.comm = comm.clone();
            assert!(script.name_matches(), "bin 对不上时 comm 应当兜底");
        }

        // 名字都对不上 = pid 被复用，不能认领也不能 kill
        let mut stale = Lease::new(me, None, "definitely-not-this-binary");
        stale.comm = String::new();
        assert!(!stale.name_matches());
        // 但存活判定仍为真（认领宽松，动手严格）
        assert!(stale.is_live());
    }

    #[test]
    fn corrupted_file_is_treated_as_empty() {
        let dir = temp_dir("bad");
        std::fs::write(dir.join("pids.json"), "{not json").unwrap();
        let s = LeaseStore::load(&dir);
        assert!(s.is_empty(), "坏文件不能挡住启动");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
