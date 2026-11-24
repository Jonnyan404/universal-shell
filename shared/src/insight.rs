//! 运行指标与事件时间线：详情页「内存占用（带曲线）」和「最近事件」的数据源。
//!
//! 以前壳只记录「是否在跑」和启动时刻，内存占用和状态变更一概不留，详情页那两张卡
//! 只能显示「—」或者干脆不画。这里补两样东西：
//!
//!   - [`Insight::sample`]：每 5 秒给运行中的子进程取一次 RSS，保留最近 [`HIST_LEN`] 个点，
//!     前端据此画 sparkline；
//!   - [`Insight::push_event`]：启停 / 异常退出 / 安装 / 自启切换各落一条事件，
//!     保留最近 [`EVENT_LEN`] 条，并持久化到 `insight.json`——重启壳之后时间线还在。
//!
//! 内存历史只放内存里：5 秒一个点写盘太吵，而且重启壳之后「上一次运行的曲线」
//! 本来也没什么参考价值。事件相反，跨重启有意义，所以单独落盘。

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 内存曲线保留的采样点数（5s 一点 → 约 2 分钟）
pub const HIST_LEN: usize = 24;
/// 每个程序保留的事件条数
pub const EVENT_LEN: usize = 12;

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Sample {
    /// unix 秒
    pub at: i64,
    /// 常驻内存（KB）
    pub rss_kb: u64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Event {
    /// unix 秒
    pub at: i64,
    /// start / stop / exit / install / autostart / supervisor
    ///
    /// 刻意**不存文案**：事件会落盘、跨重启保留，存了中文就再也切不成英文。
    /// 界面按 `kind` + `pid` / `code` 现拼（见 `workbench.js` 的 `eventText`）。
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
}

#[derive(Default)]
pub struct Insight {
    hist: BTreeMap<String, VecDeque<Sample>>,
    events: BTreeMap<String, VecDeque<Event>>,
    /// 事件落盘位置；`None` 表示纯内存（测试里用）
    path: Option<PathBuf>,
}

impl Insight {
    /// 从 `data_dir/insight.json` 读回历史事件。文件不存在/损坏都当作空，不影响启动。
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("insight.json");
        let mut me = Self {
            path: Some(path.clone()),
            ..Self::default()
        };
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(map) = serde_json::from_str::<BTreeMap<String, Vec<Event>>>(&raw) {
                for (id, evs) in map {
                    me.events.insert(id, evs.into_iter().collect());
                }
            }
        }
        me
    }

    fn persist(&self) {
        let Some(path) = &self.path else { return };
        let map: BTreeMap<&String, Vec<&Event>> =
            self.events.iter().map(|(k, v)| (k, v.iter().collect())).collect();
        if let Ok(raw) = serde_json::to_string(&map) {
            // 事件量很小（每个程序 ≤12 条），直接覆盖写；失败只影响「重启后还在不在」，
            // 不该把调用方拖垮，所以忽略错误。
            let _ = std::fs::write(path, raw);
        }
    }

    /// 记一个内存采样点，超出窗口丢最旧的。
    pub fn sample(&mut self, id: &str, rss_kb: u64) {
        let q = self.hist.entry(id.to_string()).or_default();
        q.push_back(Sample {
            at: now_secs(),
            rss_kb,
        });
        while q.len() > HIST_LEN {
            q.pop_front();
        }
    }

    /// 新一次启动：把上一轮的内存曲线清掉，sparkline 只反映这一次运行。
    pub fn clear_hist(&mut self, id: &str) {
        self.hist.remove(id);
    }

    pub fn hist(&self, id: &str) -> Vec<Sample> {
        self.hist.get(id).map(|q| q.iter().copied().collect()).unwrap_or_default()
    }

    /// 最新一次采样（KB）。没采到过返回 `None`，前端显示「—」。
    pub fn latest(&self, id: &str) -> Option<u64> {
        self.hist.get(id).and_then(|q| q.back()).map(|s| s.rss_kb)
    }

    pub fn push_event(&mut self, id: &str, kind: &str, pid: Option<u32>, code: Option<i32>) {
        let q = self.events.entry(id.to_string()).or_default();
        q.push_back(Event {
            at: now_secs(),
            kind: kind.to_string(),
            pid,
            code,
        });
        while q.len() > EVENT_LEN {
            q.pop_front();
        }
        self.persist();
    }

    pub fn events(&self, id: &str) -> Vec<Event> {
        self.events.get(id).map(|q| q.iter().cloned().collect()).unwrap_or_default()
    }

    /// 程序被删除时清掉它的曲线与事件，别在 `insight.json` 里留孤儿。
    pub fn forget(&mut self, id: &str) {
        self.hist.remove(id);
        self.events.remove(id);
        self.persist();
    }
}

/// 取进程常驻内存（KB）。取不到返回 `None`——宁可不画，也不要编一个数字。
pub fn rss_kb(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        // /proc/<pid>/statm 第二列是 resident pages
        let s = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
        let pages: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let page = if page > 0 { page as u64 } else { 4096 };
        Some(pages * page / 1024)
    }
    #[cfg(target_os = "macos")]
    {
        // macOS 没有 /proc，ps -o rss= 直接给 KB
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
    #[cfg(windows)]
    {
        // tasklist CSV 末列是 "12,345 K"
        let mut c = std::process::Command::new("tasklist");
        c.args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]);
        // 内存采样是周期性的，不隐藏控制台就会定期闪黑框
        crate::hide_console(&mut c);
        let out = c.output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().next()?;
        let mem = line.rsplit("\",\"").next()?.trim_matches('"');
        let kb: String = mem.chars().filter(|c| c.is_ascii_digit()).collect();
        kb.parse().ok()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = pid;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hist_window_is_bounded() {
        let mut me = Insight::default();
        for i in 0..(HIST_LEN + 5) {
            me.sample("a", i as u64);
        }
        let h = me.hist("a");
        assert_eq!(h.len(), HIST_LEN);
        // 最旧的被丢掉，最后一个点是最新采样
        assert_eq!(h.last().unwrap().rss_kb, (HIST_LEN + 4) as u64);
        assert_eq!(me.latest("a"), Some((HIST_LEN + 4) as u64));
    }

    #[test]
    fn events_are_bounded_and_forgettable() {
        let mut me = Insight::default();
        for i in 0..(EVENT_LEN + 3) {
            me.push_event("a", "start", Some(i as u32), None);
        }
        let evs = me.events("a");
        assert_eq!(evs.len(), EVENT_LEN);
        assert_eq!(evs.last().unwrap().pid, Some((EVENT_LEN + 2) as u32));
        me.forget("a");
        assert!(me.events("a").is_empty());
        assert!(me.latest("a").is_none());
    }

    #[test]
    fn rss_of_self_is_positive() {
        // 自己这个测试进程必然有 RSS，取不到说明实现有问题
        let kb = rss_kb(std::process::id());
        assert!(kb.unwrap_or(0) > 0, "rss_kb(self) 取不到");
    }
}
