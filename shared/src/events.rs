//! 轻量进程事件总线（F-3/F10）：shared 内的进程状态变更在此广播，
//! 界面层（web-server WS、egui、tauri）各订阅一份副本，替换高频轮询。
//!
//! 全局单例、无锁一次性订阅；多订阅者各自收到副本（内存广播）。

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// 程序已启动（显式 start 成功）
    ProgramStarted(String),
    /// 程序已停止（显式 stop 成功 / 子进程退出被清扫回收）
    ProgramStopped(String),
}

/// 壳操作日志（log_op）是否也要推——暂不需要，状态足够前端刷新。
type Sink = Arc<Mutex<Vec<Sender<Event>>>>;

fn sinks() -> &'static Sink {
    static S: OnceLock<Sink> = OnceLock::new();
    S.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

/// 订阅事件，返回接收端（每订阅者一份副本）。
pub fn subscribe() -> Receiver<Event> {
    let (tx, rx) = mpsc::channel();
    sinks().lock().unwrap().push(tx);
    rx
}

/// 广播事件到所有订阅者（对端已关闭的订阅者被剔除，避免累积占内存）。
pub fn emit(ev: Event) {
    let mut subs = sinks().lock().unwrap();
    subs.retain(|tx| tx.send(ev.clone()).is_ok());
}