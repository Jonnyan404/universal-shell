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
4. ~~子进程的 stdout 管道读端随壳进程消失，之后写日志只会 EPIPE。~~

第 4 点曾经最致命：管道读端在壳进程里，壳被 SIGKILL 后读端消失，子进程下一次
`write` 就拿到 EPIPE —— Python 抛 `BrokenPipeError`、Rust `println!` panic、
Go 默认被 `SIGPIPE` 杀掉。实测「每 5 秒 print 一次」的程序在壳死后 **5 秒内
自己退出**，租约抓到的是一具尸体。`SIGPIPE = SIG_IGN` 只挡信号，挡不住
`write()` 返回 EPIPE。

**已解决**（见第 7 节）：stdout/stderr 不再走管道，直接写文件；壳侧一个「泵」
线程轮询文件新增字节，合并进 `{id}.log` 并保留 stderr 前缀染色与实时推送。

## 3. 租约（lease）：把管理关系写到盘上

`data_dir/pids.json`：

```json
{
  "clip9": {
    "pid": 57754,
    "started_at": 1795486542,
    "port": 9511,
    "bin": "clip9-cli",
    "comm": "clip9-cli",
    "cmd_args": "-host 127.0.0.1 -port 9501"
  },
  "chatty": {
    "pid": 17038,
    "started_at": 1795490733,
    "port": null,
    "bin": "python3",
    "comm": "Python",
    "cmd_args": "/tmp/us-epipe-test/chatty.py"
  }
}
```

- `bin`：可执行文件名（`bin_path` 的最后一段）
- `comm`：实际进程名。脚本型程序这里是解释器，和 `bin` 对不上，
  结束进程时两个名字任一匹配即可
- `cmd_args`：命令行去掉 `argv[0]` 的剩余部分。**这是给「启动器再 exec 一次」
  准备的第二判据**：macOS 的 `/usr/bin/python3` 是个壳，真正跑的是
  `.../Python.app/.../Python`，进程名变成 `Python`；`argv[0]` 会被 exec 改写，
  但后面的参数原样保留，所以参数一致就能认出同一个进程
- 写入时机：`start()` 成功后；删除时机：`stop()` / 进程退出 / 删程序 / `prune`
- 自愈：`reconcile_leases()` 会用活进程 `refresh_identity()` 刷一遍这两个字段，
  旧版本写的、没有 `cmd_args` 的租约也能补上

### 认领宽松，动手严格

| 判断 | 规则 | 理由 |
|---|---|---|
| `Lease::is_live` | 只看 `kill(pid, 0)` | 名字对不上可能是脚本，一律当死会把正常残留丢掉 |
| `Lease::name_matches` | `bin`/`comm` 匹配当前进程名，**或** `cmd_args` 与当前命令行参数完全一致 | pid 可能被复用，kill 前必须确认；启动器 exec 会换掉进程名，参数不会变 |
| `terminate` | `name_matches` 为假就拒绝 | 宁可清理失败，也不能误杀无关进程 |

### 启动时 reconcile

`web-server::start_with_state()` 在起服务前调用 `mgr.reconcile_leases()`：

1. `prune()` 清掉死记录；
2. `refresh_live()` 用活进程刷新 `comm` / `cmd_args`，活着的写进 `external: BTreeMap<String, u32>`；
   顺手 `logpump::watch(..., resume = true)` 把日志泵接上（孤儿进程的输出照样实时进界面）；
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

- 进程组：Unix 上 `setsid` + 按 pgid 杀，连带清掉程序自己 fork 的子进程
- 外部接管的进程不采样内存（没有 pid 之外的凭据时无法确认归属，先不采）
- Windows 上 `pid_cmd_args` 取不到命令行（`tasklist` 不提供），只能靠进程名认人

---

## 7. 日志：不用管道，改「文件 + 合并泵」

代码在 `shared/src/logpump.rs`。

### 为什么管道不行

管道的读端属于壳进程。壳被 SIGKILL → 读端关闭 → 子进程 `write` 返回 EPIPE。
这不是「偶尔报个错」，是**把子进程直接弄死**（见第 2 节第 4 点）。

### 现在的结构

```
子进程 stdout ──► {id}.stdout  ┐
                              ├─► 泵线程（壳内，120ms 轮询）──► {id}.log ──► 界面
子进程 stderr ──► {id}.stderr  ┘        stderr 行加 \x1F 前缀
```

- `open_sources()` 用 `append` 打开两个普通文件，直接作为 `Stdio` 交给 `Command`。
  壳怎么死，子进程的 `write` 都成功。
- 泵线程每 120ms 读两个文件的新增字节（记住偏移），把 stdout 原样、stderr
  逐行加 `\x1F` 前缀后追加进合并日志 `{id}.log`，然后广播 `LogWritten` 事件
  （前端据此增量拉 `get_logs`，150ms 内合并成一次）。
- 偏移落盘在 `{id}.log.off`（`"<out> <err>"`，临时文件 + rename）。
  壳重启后从上次偏移继续，**壳死掉那段时间的输出会被补进合并日志**。
- 泵**不依赖子进程句柄**，只读文件。所以孤儿进程（外部接管）也能继续采集日志，
  `reconcile_leases()` 接管时顺手 `watch(..., resume=true)`。
- 截断：合并日志 512KB、源文件 4MB，超限截末一半并对齐行首。
- `clear_log` 必须连两个源文件一起清，否则泵下一轮会把旧内容再合一遍。

### 代价（如实记录）

- **两路之间没有严格时序**：同一轮轮询里先追加 stdout 再追加 stderr。
  正常速率下（每轮各一行）看起来是交织的；如果壳刚重启、积压了几十行，
  会看到「一段 stdout + 一段 stderr」。旧实现是两个线程抢同一把写锁，
  本来也没有严格时序保证，所以这不算退步。
- **壳死掉期间源文件没人裁**。合并日志仍按 512KB 截，源文件在泵恢复后
  一并裁到 4MB。壳长时间不启动 + 程序狂打日志，磁盘会被吃掉一些。

### 验证过的场景

1. 起一个每秒写 stdout+stderr 的 python 程序 → 合并日志里 `stdout tick N` 与
   `\x1Fstderr tick N` 逐行交织，界面 stderr 行是红色。
2. `kill -9` 壳 → 子进程 PPID=1 **继续存活并继续写**（旧实现在 1 秒内就死了）。
3. 重启壳 → `workspace_list` 返回 `external_pid`，泵从 `{id}.log.off` 继续，
   把壳死掉那段的输出补进合并日志（实测 tick 29–32 补齐，无重复无丢失）。
4. 界面不刷新，4 秒内日志从 tick 87 涨到 91 —— 实时推送链路完好。
5. `stop_program` → 进程结束、泵收尾。

### 顺带修掉的两个坑

- **`working_dir` 为空串**时 `Command::spawn` 返回 ENOENT，错误信息却指着二进制
  （「Cannot start /usr/bin/python3: No such file or directory」），极难排查。
  现在空串按 `.` 处理，自定义目录不存在则给出明确报错。
- **进程名被 exec 换掉**：macOS 的 `/usr/bin/python3` 是个壳，真正跑的是
  `.../Python.app/.../Python`，进程名变成 `Python`。租约里记的是 `python3`，
  于是「停止」以「pid 可能已被复用」为由拒绝，孤儿永远清不掉。
  租约新增 `cmd_args`（命令行去掉 argv[0]）：**进程名对不上时看参数**，
  参数完全一致即认作同一个进程。`reconcile_leases()` 接管时会用活进程刷新
  这两个字段，旧版本写的、没有 `cmd_args` 的租约也能自愈。
