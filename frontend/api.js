// Universal Shell 浏览器端 RPC 层。
// 语义对齐 Tauri 的 `invoke`：同一份前端逻辑，请求走本地 HTTP /api/rpc。
// 本文件是唯一与传输层相关的代码（F-2 起可在此追加 token、WS 事件订阅）。

globalThis.invoke = async function invoke(cmd, args) {
  const res = await fetch("/api/rpc", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ cmd, args: args || {} }),
  });
  if (!res.ok) throw new Error("HTTP " + res.status);
  const body = await res.json();
  if (!body.ok) throw new Error(body.error || "rpc error: " + cmd);
  return body.data;
};