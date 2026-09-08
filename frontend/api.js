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

globalThis.invoke = async function invoke(cmd, args) {
  const headers = { "Content-Type": "application/json" };
  if (usToken) headers["X-Universal-Token"] = usToken;
  const res = await fetch("/api/rpc", {
    method: "POST",
    headers,
    body: JSON.stringify({ cmd, args: args || {} }),
  });
  if (!res.ok) throw new Error("HTTP " + res.status);
  const body = await res.json();
  if (!body.ok) throw new Error(body.error || "rpc error: " + cmd);
  return body.data;
};