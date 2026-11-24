// Universal Shell 浏览器端 RPC 层。
// 语义对齐 Tauri 的 `invoke`：同一份前端逻辑，请求走本地 HTTP /api/rpc。
// 本文件是唯一与传输层相关的代码（F-2 起可在此追加 token、WS 事件订阅）。

// F6 鉴权：非回环访问需携带令牌。令牌来自 URL ?token= 或本地缓存（同一源秒级内有效）。
const TOKEN_STORE = "us_token:" + location.host;
let usToken =
  localStorage.getItem(TOKEN_STORE) ||
  new URLSearchParams(location.search).get("token") ||
  "";
if (usToken) localStorage.setItem(TOKEN_STORE, usToken);
globalThis.usToken = usToken;

// 默认超时。没有它的话，一次挂住的请求会让调用方的 `await` 永远不返回：
// 切换程序时 get_values 卡住，配置页就会一直显示上一个程序的字段
// （实测无外网时页面加载的 registry 请求占满浏览器连接池，能卡十几秒）。
const RPC_TIMEOUT_MS = 20000;

globalThis.invoke = async function invoke(cmd, args, opts) {
  const headers = { "Content-Type": "application/json" };
  if (usToken) headers["X-Universal-Token"] = usToken;
  // 下载 / 拉清单 / 联网检查这类天然慢的操作由调用方显式放宽
  const timeout = (opts && opts.timeout) || RPC_TIMEOUT_MS;
  const ac = new AbortController();
  const timer = setTimeout(() => ac.abort(), timeout);
  try {
    const res = await fetch("/api/rpc", {
      method: "POST",
      headers,
      body: JSON.stringify({ cmd, args: args || {} }),
      signal: ac.signal,
    });
    if (!res.ok) throw new Error("HTTP " + res.status);
    const body = await res.json();
    if (!body.ok) throw new Error(body.error || "rpc error: " + cmd);
    return body.data;
  } catch (e) {
    if (e && e.name === "AbortError") {
      throw new Error(`${cmd} 超时（${Math.round(timeout / 1000)}s 未响应）`);
    }
    throw e;
  } finally {
    clearTimeout(timer);
  }
};