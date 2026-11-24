# 进程生命周期：归属、孤儿与接管

本文记录「壳如何管理它启动的程序」这件事的设计与边界。改动涉及
`shared/src/lease.rs`、`shared/src/runner.rs`、`shared/src/shell_manager.rs`、
`web-server/src/{lib,rpc}.rs`。

## 1. 两种归属

| | managed（壳托管） | native（系统调度） |
|---|---|---|
| 谁 spawn | `Runner::start_async` | launchd / systemd / 计划任务 |
| 壳持有的凭据 | `Child` 句柄（内存） | 无 |
| 状态来源 | `Runner.children` 表 | `launchctl list` / `systemctl is-active` / `schtasks /Query` |
| 壳重启后 | 失联（靠租约找回） | 不受影响，状态照常准确 |
| 停止方式 | `Child::kill()` | `us_service::stop()` |
| uptime | 有 | `null`（壳看不到启动时刻） |

native 的关键是**状态真相不在壳里**：`us_service::state()` 每次都去问操作系统，
壳只是个遥控器，重启一百次也不影响。所以需要长期存活的程序应当走 native。

## 2. managed 的断点：壳被 SIGKILL

```
ShellManager::start()
  → Runner::start_async()  spawn，stdout/stderr 走 pipe
  → children: BTreeMap<String, Child>   ← 唯一凭据，纯内存
```

`impl Drop for Runner` 会在壳正常退出时 `stop_all()`。但 **Drop 只覆盖正常路径**：

| 退出方式 | Drop 执行 | 结果 |
|---|---|---|
| Ctrl-C / 托盘退出 | 是 | 干净 |
| panic（默认 unwind） | 是 | 干净 |
| SIGKILL / 段错误 / 断电 / 系统重启 | **否** | **孤儿，PPID=1** |

孤儿会造成一连串问题：

1. 端口、数据库文件锁（如 redb）仍被占用；
2. 壳重启后 `children` 表空 → 界面显示「未运行」；
3. 用户点启动 → 新实例打开同一个数据库 → `Cannot acquire lock` → exit 1；
4. 子进程的 stdout 管道读端随壳进程消失，之后写日志只会 EPIPE。

第 4 点比想象中严重：**持续往 stdout 写日志的程序会在壳死后自己退出**
（Python 抛 `BrokenPipeError`、Rust `println!` panic）。所以租约对「启动后
安静运行」的程序有效（例如 clip9 这种 Rust 二进制），对高频打印的程序只能
在它自杀前的一小段时间内有效。

> 根治办法是让子进程的 stdout/stderr 直接指向日志文件（fd 不经过壳），
> 代价是失去 stderr 前缀染色与实时推送。尚未实施。

## 3. 租约（lease）：把管理关系写到盘上

`data_dir/pids.json`：

```json
{
  "clip9": {
    "pid": 57754,
    "started_at": 1795486542,
    "port": 9511,
    "bin": "clip9-cli",
    "comm": "python3"
  }
}
```

- `bin`：可执行文件名（`bin_path` 的最后一段）
- `comm`：spawn 之后实际进程名。脚本型程序这里是解释器，和 `bin` 对不上，
  结束进程时两个名字任一匹配即可
- 写入时机：`start()` 成功后；删除时机：`stop()` / 进程退出 / 删程序 / `prune`

### 认领宽松，动手严格

| 判断 | 规则 | 理由 |
|---|---|---|
| `Lease::is_live` | 只看 `kill(pid, 0)` | 名字对不上可能是脚本，一律当死会把正常残留丢掉 |
| `Lease::name_matches` | `bin` 或 `comm` 任一匹配当前进程名 | pid 可能被复用，kill 前必须确认 |
| `terminate` | `name_matches` 为假就拒绝 | 宁可清理失败，也不能误杀无关进程 |

### 启动时 reconcile

`web-server::start_with_state()` 在起服务前调用 `mgr.reconcile_leases()`：

1. `prune()` 清掉死记录；
2. 活着的写进 `external: BTreeMap<String, u32>`；
3. 打一条 warn 日志「接管 N 个上次会话残留进程」。

之后：

- `is_program_running()` = 有句柄 **或** 在 external 表里
- 列表接口每项多一个 `external_pid`，前端据此显示琥珀色「外部接管」胶囊
- 详情页概览的「运行时长」KPI 显示 `PID xxxxx` 而不是「—」
- `stop()` 没有句柄时走租约分支：`terminate()` 先 SIGTERM 等 3 秒，再 SIGKILL
- `restart_program` 先判 `is_program_running`，避免「本来没跑」导致重启失败
- sweep 线程每 500ms 调 `reap_external()`：外部进程死了就撤状态、写退出事件

## 4. 启动前预检

`start()` 里两道拦截，都发生在 spawn 之前：

1. **租约残留**：`leases.get(id)` 存在且 `is_live()` →
   `「clip9」上次会话的进程还在运行（PID 57754），壳没有它的句柄。先「停止」结束它再启动。`
2. **端口占用**：`declared_port()` 取 `native.ready_port` 或字段 `port`，
   `lease::port_owner()` 用 `lsof -nP -iTCP:<port> -sTCP:LISTEN -t`（Windows
   `netstat -ano`）查占用者 →
   `端口 9501 已被 PID 27669 占用（不是本壳启动的进程）。先结束它，或改「clip9」的端口。`

预检是 best-effort：探测失败（没装 lsof、无权限）就跳过，不能因此拦住启动。

## 5. 排查手册

```bash
# 有没有残留进程
cat "$HOME/Library/Application Support/universal-shell/pids.json"

# 谁占着端口
lsof -nP -iTCP:9501 -sTCP:LISTEN

# 手工清一条租约（进程已死但记录还在时）
#   直接编辑 pids.json 删掉那个 key，或等壳下次启动时自动 prune
```

## 6. 仍未做

- 子进程 stdout/stderr 改指向文件（解决 EPIPE 自杀）
- 进程组：Unix 上 `setsid` + 按 pgid 杀，连带清掉程序自己 fork 的子进程
- 外部接管的进程不采样内存（没有 pid 之外的凭据时无法确认归属，先不采）
