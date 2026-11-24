//! 子进程日志落盘与合并：**不用管道**。
//!
//! ## 为什么不能再用管道
//!
//! 旧实现是 `Stdio::piped()` + 两个读线程，把 stdout/stderr 写进同一个
//! `{id}.log`，stderr 行加 `\x1F` 前缀供前端染色。问题是**管道的读端在壳进程里**：
//! 壳被 `SIGKILL`（Drop 不执行）后读端随进程消失，子进程下一次 `write` 就拿到
//! `EPIPE` —— Python 抛 `BrokenPipeError`、Rust 的 `println!` 直接 panic、
//! Go 默认被 `SIGPIPE` 杀掉。实测「每 5 秒 print 一次」的假 clip9 在壳死后
//! **5 秒内自己退出**，租约抓到的是一具尸体，孤儿接管形同虚设。
//! `SIGPIPE = SIG_IGN` 只挡信号，挡不住 `write()` 返回 EPIPE。
//!
//! ## 现在的做法
//!
//! 子进程的 stdout/stderr **直接指向两个普通文件**（`{id}.stdout` / `{id}.stderr`，
//! append 模式）。壳里跑一个「泵」线程，轮询这两个文件的新增字节，把 stderr 的
//! 每一行加上 `\x1F` 前缀后追加进合并日志 `{id}.log`。于是：
//!
//! - 壳死了，泵跟着停，但子进程的 `write` **永远成功**，不会再自杀；
//! - 壳重启后泵从 `{id}.log.off` 记下的偏移继续，把壳死掉那段时间的输出补进合并日志；
//! - 合并日志仍是单一文件，`get_logs` 的按偏移增量读取、截断策略、前端染色全部不变；
//! - 泵不依赖子进程句柄，**孤儿进程（外部接管）也能继续采集日志**。
//!
//! ## 代价（如实记录）
//!
//! 两路之间不再有严格时序：同一轮轮询里先追加 stdout 再追加 stderr。但旧实现是
//! 两个线程抢同一把写锁，本来也没有时序保证，所以这不算退步。真正的顺序信息在
//! 程序自己打的日志里（大多数库会带时间戳）。
//!
//! 另一个代价：源文件（`{id}.stdout` / `{id}.stderr`）在壳死掉期间没人裁剪。
//! 合并日志仍按 512KB 截末一半，源文件在泵恢复后会被一并裁到上限。

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 合并日志单文件上限（字节），超限截末一半
pub const MERGED_MAX: u64 = 512 * 1024;
/// 源文件上限：比合并日志宽松些，留出「壳死掉期间」的缓冲
pub const SOURCE_MAX: u64 = 4 * 1024 * 1024;
/// 轮询间隔。120ms 对肉眼就是实时，又不会把 CPU 烧掉
const TICK: Duration = Duration::from_millis(120);
/// 单轮每路最多吞多少字节，防止一次卡住太久
const CHUNK: usize = 256 * 1024;
/// log-tick 事件最小发射间隔（与旧实现一致）
const EMIT_INTERVAL: Duration = Duration::from_millis(150);

/// stderr 行前缀标记（前端 `logLineEl` 认这个字符给整行上错误色）
pub const STDERR_MARK: u8 = 0x1f;

pub fn merged_path(log_dir: &Path, id: &str) -> PathBuf {
    log_dir.join(format!("{id}.log"))
}
pub fn stdout_path(log_dir: &Path, id: &str) -> PathBuf {
    log_dir.join(format!("{id}.stdout"))
}
pub fn stderr_path(log_dir: &Path, id: &str) -> PathBuf {
    log_dir.join(format!("{id}.stderr"))
}
fn offset_path(log_dir: &Path, id: &str) -> PathBuf {
    log_dir.join(format!("{id}.log.off"))
}

/// 打开（或创建）子进程 stdout/stderr 的目标文件。子进程直接写这两个句柄，
/// 全程不经过壳，所以壳怎么死都不会 EPIPE。
pub fn open_sources(log_dir: &Path, id: &str) -> std::io::Result<(File, File)> {
    std::fs::create_dir_all(log_dir)?;
    let open = |p: PathBuf| -> std::io::Result<File> {
        OpenOptions::new().create(true).append(true).open(p)
    };
    Ok((open(stdout_path(log_dir, id))?, open(stderr_path(log_dir, id))?))
}

/// 新开一轮运行时把偏移重置到源文件当前末尾：上一轮已经合并过的内容不能再合一遍。
pub fn reset_offsets(log_dir: &Path, id: &str) {
    let out = std::fs::metadata(stdout_path(log_dir, id))
        .map(|m| m.len())
        .unwrap_or(0);
    let err = std::fs::metadata(stderr_path(log_dir, id))
        .map(|m| m.len())
        .unwrap_or(0);
    save_offsets(log_dir, id, out, err);
}

fn save_offsets(log_dir: &Path, id: &str, out: u64, err: u64) {
    // 临时文件 + rename：写一半被 kill 时不会留下坏偏移
    let path = offset_path(log_dir, id);
    let tmp = path.with_extension("off.tmp");
    if std::fs::write(&tmp, format!("{out} {err}")).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn load_offsets(log_dir: &Path, id: &str) -> Option<(u64, u64)> {
    let text = std::fs::read_to_string(offset_path(log_dir, id)).ok()?;
    let mut it = text.split_whitespace();
    let out = it.next()?.parse().ok()?;
    let err = it.next()?.parse().ok()?;
    Some((out, err))
}

/// 一路源的读取状态
#[derive(Default)]
struct Stream {
    path: PathBuf,
    off: u64,
    /// 上一次读到的字节是不是行尾。stderr 只在行首插 `\x1F`，
    /// 半行续写不能再插一次，否则一行会被拆成「错误色 + 普通色」两段。
    at_line_start: bool,
}

struct Watched {
    merged: PathBuf,
    out: Stream,
    err: Stream,
    /// 泵刚接管、还没读过任何东西：第一轮只记录偏移，不追加。
    /// 用于「壳重启接管孤儿」时避免把上一轮已经合并过的内容再合一遍。
    prime: bool,
    last_emit: Option<Instant>,
}

static WATCHED: OnceLock<Mutex<BTreeMap<String, Watched>>> = OnceLock::new();
static STARTED: AtomicBool = AtomicBool::new(false);

fn watched() -> &'static Mutex<BTreeMap<String, Watched>> {
    WATCHED.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// 开始采集某个程序的日志。
///
/// - `resume = false`（新起一轮）：偏移对齐到源文件当前末尾，只合并本次运行新写的；
/// - `resume = true`（壳重启接管孤儿）：优先用上次落盘的偏移，把壳死掉那段的输出补齐。
///
/// 幂等：同一个 id 重复调用不会起第二个泵。
pub fn watch(log_dir: &Path, id: &str, resume: bool) {
    let mut map = watched().lock().unwrap();
    if let Some(w) = map.get_mut(id) {
        // 已经在泵了。`resume = false` 说明是新起一轮：必须把偏移重新对齐到
        // 源文件当前末尾，否则泵还拿着上一轮的内存偏移 —— 若新文件比旧偏移短，
        // `read_new` 会当成「文件被截短」从头重读，把上一轮内容再合一遍。
        if !resume {
            w.out.off = std::fs::metadata(&w.out.path).map(|m| m.len()).unwrap_or(0);
            w.err.off = std::fs::metadata(&w.err.path).map(|m| m.len()).unwrap_or(0);
            w.out.at_line_start = true;
            w.err.at_line_start = true;
            w.prime = false;
        }
        return;
    }
    let saved = if resume { load_offsets(log_dir, id) } else { None };
    let (out_off, err_off) = saved.unwrap_or_else(|| {
        (
            std::fs::metadata(stdout_path(log_dir, id)).map(|m| m.len()).unwrap_or(0),
            std::fs::metadata(stderr_path(log_dir, id)).map(|m| m.len()).unwrap_or(0),
        )
    });
    map.insert(
        id.to_string(),
        Watched {
            merged: merged_path(log_dir, id),
            out: Stream {
                path: stdout_path(log_dir, id),
                off: out_off,
                at_line_start: true,
            },
            err: Stream {
                path: stderr_path(log_dir, id),
                off: err_off,
                at_line_start: true,
            },
            prime: saved.is_none(),
            last_emit: None,
        },
    );
    drop(map);
    ensure_thread();
}

/// 停止采集。退出前做最后一次抽取，别把进程退出前那几行丢掉。
pub fn unwatch(id: &str) {
    let mut map = watched().lock().unwrap();
    if let Some(mut w) = map.remove(id) {
        let mut grew = false;
        pump(&mut w, &mut grew);
    }
}

/// 是否正在采集（测试与排查用）
pub fn is_watching(id: &str) -> bool {
    watched().lock().unwrap().contains_key(id)
}

fn ensure_thread() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("us-logpump".into())
        .spawn(|| loop {
            std::thread::sleep(TICK);
            let mut grown: Vec<String> = Vec::new();
            {
                let mut map = watched().lock().unwrap();
                let now = Instant::now();
                for (id, w) in map.iter_mut() {
                    let mut grew = false;
                    pump(w, &mut grew);
                    if grew {
                        let due = match w.last_emit {
                            None => true,
                            Some(t) => t.elapsed() >= EMIT_INTERVAL,
                        };
                        if due {
                            w.last_emit = Some(now);
                            grown.push(id.clone());
                        }
                    }
                }
            }
            // 事件广播放在锁外：emit 会锁事件总线的 sink 表，
            // 在泵的锁里做的话，一个卡住的订阅者会把所有程序一起拖死。
            for id in grown {
                crate::events::emit(crate::events::Event::LogWritten(id));
            }
        })
        .ok();
}

fn save_offsets_of(w: &Watched) {
    let Some(dir) = w.merged.parent() else { return };
    let Some(id) = w.merged.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
        return;
    };
    save_offsets(dir, &id, w.out.off, w.err.off);
}

/// 抽一轮。返回是否追加了内容。
fn pump(w: &mut Watched, grew: &mut bool) {
    let out_bytes = read_new(&mut w.out);
    let err_bytes = read_new(&mut w.err);
    if w.prime {
        // 第一轮只对齐偏移：老内容可能已经在合并日志里了
        w.prime = false;
        return;
    }
    if out_bytes.is_empty() && err_bytes.is_empty() {
        return;
    }
    let mut buf: Vec<u8> = Vec::with_capacity(out_bytes.len() + err_bytes.len() + 64);
    buf.extend_from_slice(&out_bytes);
    // stderr：逐行加前缀。只在实际行首加，半行续写跳过。
    let mut at_start = w.err.at_line_start;
    for &b in &err_bytes {
        if at_start {
            buf.push(STDERR_MARK);
        }
        buf.push(b);
        at_start = b == b'\n';
    }
    w.err.at_line_start = at_start;

    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&w.merged) {
        if f.write_all(&buf).is_ok() && f.flush().is_ok() {
            *grew = true;
        }
    }
    // 合并日志按 512KB 截；源文件上限放宽到 4MB，只在壳死掉那段时间没人裁
    trim_file(&w.merged, MERGED_MAX);
    trim_file(&w.out.path, SOURCE_MAX);
    trim_file(&w.err.path, SOURCE_MAX);
    save_offsets_of(w);
}

/// 从上次偏移读到文件末尾。文件被外部截短（清空 / 轮转）时把偏移拉回 0 重读。
fn read_new(s: &mut Stream) -> Vec<u8> {
    let Ok(mut f) = File::open(&s.path) else {
        return Vec::new();
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return Vec::new();
    };
    if len < s.off {
        // 文件被清空或截短：从头再来
        s.off = 0;
        s.at_line_start = true;
    }
    if len == s.off {
        return Vec::new();
    }
    if f.seek(SeekFrom::Start(s.off)).is_err() {
        return Vec::new();
    }
    let want = ((len - s.off) as usize).min(CHUNK);
    let mut buf = vec![0u8; want];
    let mut got = 0usize;
    while got < want {
        match f.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(_) => break,
        }
    }
    buf.truncate(got);
    s.off += got as u64;
    buf
}

/// 体积超限时保留末尾一半的完整行。和 `runner::trim_program_log` 同策略，
/// 但这里按路径操作（源文件由子进程持有句柄，我们只能截不能重写它）。
pub fn trim_file(path: &Path, max: u64) {
    let Ok(mut f) = OpenOptions::new().read(true).write(true).open(path) else {
        return;
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return;
    };
    if len <= max {
        return;
    }
    let keep_from = len.saturating_sub(max / 2);
    if f.seek(SeekFrom::Start(keep_from)).is_err() {
        return;
    }
    let mut tail = Vec::new();
    if f.read_to_end(&mut tail).is_err() {
        return;
    }
    // 对齐到下一行行首，避免第一行是半行
    let start = tail
        .iter()
        .position(|&b| b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let keep = &tail[start.min(tail.len())..];
    if f.set_len(0).is_err() || f.seek(SeekFrom::Start(0)).is_err() {
        return;
    }
    let _ = f.write_all(keep);
    let _ = f.flush();
}

/// 清空某程序的全部日志（合并日志 + 两个源文件 + 偏移）。
/// 源文件必须一起清：只清合并日志的话，泵下一轮会把源文件里的旧内容再合一遍。
pub fn clear(log_dir: &Path, id: &str) {
    for p in [
        merged_path(log_dir, id),
        stdout_path(log_dir, id),
        stderr_path(log_dir, id),
    ] {
        let _ = std::fs::write(&p, b"");
    }
    save_offsets(log_dir, id, 0, 0);
    if let Some(w) = watched().lock().unwrap().get_mut(id) {
        w.out.off = 0;
        w.err.off = 0;
        w.out.at_line_start = true;
        w.err.at_line_start = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cc-logpump-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 源文件新增的内容被抽进合并日志，stderr 行带 \x1F 前缀。
    #[test]
    fn pumps_both_streams_with_stderr_mark() {
        let dir = tmpdir("basic");
        let id = "app";
        let (mut out, mut err) = open_sources(&dir, id).unwrap();
        // 偏移对齐到 0（源文件是空的，reset 后就是 0）
        reset_offsets(&dir, id);
        watch(&dir, id, false);
        // prime 那一轮只对齐偏移，直接手工构造 Watched 更可控
        unwatch(id);

        let mut w = Watched {
            merged: merged_path(&dir, id),
            out: Stream {
                path: stdout_path(&dir, id),
                off: 0,
                at_line_start: true,
            },
            err: Stream {
                path: stderr_path(&dir, id),
                off: 0,
                at_line_start: true,
            },
            prime: false,
            last_emit: None,
        };
        out.write_all(b"hello\nworld\n").unwrap();
        out.flush().unwrap();
        err.write_all(b"boom\n").unwrap();
        err.flush().unwrap();

        let mut grew = false;
        pump(&mut w, &mut grew);
        assert!(grew, "有新增内容就该报告 grew");

        let text = std::fs::read_to_string(merged_path(&dir, id)).unwrap();
        assert!(text.contains("hello\n"), "stdout 原样写入：{text:?}");
        assert!(
            text.contains("\u{1f}boom\n"),
            "stderr 行要带 \\x1F 前缀：{text:?}"
        );
        assert!(!text.contains("\u{1f}hello"), "stdout 不该被标成 stderr");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 半行续写：第一段没有换行，第二段开头不能再插一次标记。
    #[test]
    fn stderr_mark_only_at_line_start() {
        let dir = tmpdir("partial");
        let id = "app";
        let (_out, mut err) = open_sources(&dir, id).unwrap();
        let mut w = Watched {
            merged: merged_path(&dir, id),
            out: Stream {
                path: stdout_path(&dir, id),
                off: 0,
                at_line_start: true,
            },
            err: Stream {
                path: stderr_path(&dir, id),
                off: 0,
                at_line_start: true,
            },
            prime: false,
            last_emit: None,
        };
        err.write_all(b"par").unwrap();
        err.flush().unwrap();
        let mut grew = false;
        pump(&mut w, &mut grew);
        err.write_all(b"tial\n").unwrap();
        err.flush().unwrap();
        pump(&mut w, &mut grew);

        let text = std::fs::read_to_string(merged_path(&dir, id)).unwrap();
        assert_eq!(text, "\u{1f}partial\n", "半行只能有一个前缀：{text:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 偏移能落盘、能读回；壳重启后接着上次的位置读，不重复也不丢。
    #[test]
    fn offsets_survive_restart() {
        let dir = tmpdir("offsets");
        let id = "app";
        let (mut out, _err) = open_sources(&dir, id).unwrap();
        out.write_all(b"first\n").unwrap();
        out.flush().unwrap();
        save_offsets(&dir, id, 6, 0);
        assert_eq!(load_offsets(&dir, id), Some((6, 0)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 超限截末一半、行对齐。
    #[test]
    fn trim_keeps_tail_half() {
        let dir = tmpdir("trim");
        let path = dir.join("big.log");
        let mut s = String::new();
        for i in 0..20000 {
            s.push_str(&format!("line-{i:05}-0123456789abcdef\n"));
        }
        std::fs::write(&path, &s).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > MERGED_MAX);
        trim_file(&path, MERGED_MAX);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!((text.len() as u64) < MERGED_MAX);
        assert!(text.ends_with("line-19999-0123456789abcdef\n"));
        assert!(!text.contains("line-00000-"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
