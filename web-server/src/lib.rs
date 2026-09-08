//! universal-shell 本地 Web 管理服务（库形态）。
//!
//! 不单独打包：由宿主进程（egui / Tauri）内嵌启动，绑定 127.0.0.1 的一个高位端口，
//! 供浏览器与桌面窗口访问同一份 SPA。前端资源在编译期内嵌进本 crate。

pub mod rpc;

pub use rpc::enable_native_pick;

rust_i18n::i18n!("../shared/locales");

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as PathParam, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;

use shared::ShellManager;

pub use rpc::RpcState;

const SPA_INDEX: &str = include_str!("../../frontend/index.html");
const SPA_STYLES: &str = include_str!("../../frontend/styles.css");
const SPA_MAIN_JS: &str = include_str!("../../frontend/main.js");
const SPA_API_JS: &str = include_str!("../../frontend/api.js");
const LOCALE_ZH: &str = include_str!("../../frontend/locales/zh-CN.json");
const LOCALE_EN: &str = include_str!("../../frontend/locales/en.json");

/// 运行中的服务句柄：记录实际端口、展示用 URL，并提供停止能力（用于 egui 开关关闭）。
pub struct WebServerHandle {
    pub port: u16,
    /// 给用户看的访问地址（当前仅 loopback 模式，F-2 起支持 LAN/token）
    pub url: String,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl WebServerHandle {
    /// 停止服务并等待线程退出（幂等）
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }

    /// 浏览器打开入口。返回是否成功（无浏览器环境时失败可忽略）。
    pub fn open_in_browser(&self) -> std::io::Result<()> {
        open_url(&self.url)
    }
}

/// 启动内嵌 web 服务。
///
/// - `bind`：监听地址（默认 `127.0.0.1`；LAN 需显式传入非 loopback 并由调用方做权限控制）
/// - `port`：`0` = 自动选随机空闲高位端口
///
/// 阻塞至绑定完成，返回实际端口；之后在后台线程持续服务。
pub fn start(
    manager: Arc<Mutex<ShellManager>>,
    config_path: PathBuf,
    bind: &str,
    port: u16,
) -> anyhow::Result<WebServerHandle> {
    let state = Arc::new(RpcState::new(manager, config_path));
    start_with_state(state, bind, port)
}

/// 首选指定端口；占用时回退自动分配（F7 端口配置的容错启动）。
pub fn start_preferred(
    manager: Arc<Mutex<ShellManager>>,
    config_path: PathBuf,
    bind: &str,
    port: u16,
) -> anyhow::Result<WebServerHandle> {
    if port != 0 {
        match start(manager.clone(), config_path.clone(), bind, port) {
            Ok(h) => return Ok(h),
            Err(e) => {
                log::warn!("web-server: preferred port {port} unavailable ({e:#}); auto-assigning");
            }
        }
    }
    start(manager, config_path, bind, 0)
}

/// 同 [`start`]，但接受调用方已构造好的状态（便于宿主进程把同一含锁 manager 共享进来）。
pub fn start_with_state(
    state: Arc<RpcState>,
    bind: &str,
    port: u16,
) -> anyhow::Result<WebServerHandle> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;

    let (port_tx, port_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_handle = stop.clone();
    let bind_addr = format!("{bind}:{port}");

    let join = std::thread::Builder::new()
        .name("us-web".to_string())
        .spawn(move || {
            rt.block_on(async move {
                let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
                    Ok(l) => l,
                    Err(e) => {
                        log::error!("web-server: bind {bind_addr} failed: {e:#}");
                        let _ = port_tx.send(Err(anyhow::anyhow!(e)));
                        return;
                    }
                };
                let addr: SocketAddr = listener.local_addr().map_err(anyhow::Error::from).expect("local_addr");
                let _ = port_tx.send(Ok(addr.port()));

                // F-3(F10)：状态事件 → WS 推送（替换浏览器轮询）。
                //   - sweep 线程：定期清扫已退出的子进程（外部 kill / 崩溃），emit 到 shared 事件总线；
                //   - 转发线程：订阅事件总线，翻译成 WS JSON 广播。
                // 共用 stop 标志，服务停止即退出，避免每次开关启动时泄漏线程。
                let sweep_stop = stop.clone();
                let sweep_state = state.clone();
                std::thread::Builder::new()
                    .name("us-sweep".to_string())
                    .spawn(move || {
                        while !sweep_stop.load(Ordering::SeqCst) {
                            let exited = {
                                let mut mgr = sweep_state.manager.lock().unwrap();
                                mgr.sweep_exited_programs()
                            };
                            for id in exited {
                                shared::events::emit(shared::events::Event::ProgramStopped(id));
                            }
                            std::thread::sleep(std::time::Duration::from_millis(500));
                        }
                    })
                    .expect("spawn us-sweep");
                let fwd_stop = stop.clone();
                let fwd_state = state.clone();
                std::thread::Builder::new()
                    .name("us-events".to_string())
                    .spawn(move || {
                        let rx = shared::events::subscribe();
                        loop {
                            match rx.recv_timeout(std::time::Duration::from_millis(300)) {
                                Ok(ev) => {
                                    let (id, running) = match ev {
                                        shared::events::Event::ProgramStarted(id) => (id, true),
                                        shared::events::Event::ProgramStopped(id) => (id, false),
                                    };
                                    let msg = serde_json::json!({
                                        "type": "status-change",
                                        "programId": id,
                                        "running": running,
                                    })
                                    .to_string();
                                    let _ = fwd_state.events.send(msg);
                                }
                                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                                Err(mpsc::RecvTimeoutError::Timeout) => {
                                    if fwd_stop.load(Ordering::SeqCst) {
                                        return;
                                    }
                                }
                            }
                        }
                    })
                    .expect("spawn us-events");

                let app = Router::new()
                    .route("/", get(index))
                    .route("/styles.css", get(styles))
                    .route("/main.js", get(main_js))
                    .route("/api.js", get(api_js))
                    .route("/locales/:lang", get(locale))
                    .route("/api/rpc", post(rpc_handler))
                    .route("/api/capabilities", get(capabilities))
                    .route("/ws", get(ws_handler))
                    // F6 鉴权：回环 peer 免 token；其它来源必须携带 token（查询参数或 X-Universal-Token 头）
                    .route_layer(
                        axum::middleware::from_fn_with_state(state.clone(), web_auth)
                    )
                    .with_state(state);

                let shutdown = {
                    let stop = stop.clone();
                    async move {
                        while !stop.load(Ordering::SeqCst) {
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        }
                    }
                };
                log::info!("web-server listening on http://{addr}");
                if let Err(e) = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
                    .with_graceful_shutdown(shutdown)
                    .await
                {
                    log::error!("web-server: {e:#}");
                }
            })
        })?;

    match port_rx.recv_timeout(std::time::Duration::from_secs(10))? {
        Ok(port) => {
            let url = format!("http://127.0.0.1:{port}");
            Ok(WebServerHandle { port, url, stop: stop_handle, join: Some(join) })
        }
        Err(e) => Err(anyhow::anyhow!("web-server failed to start: {e:#}")),
    }
}

// -------- 静态资源 --------

async fn index() -> impl IntoResponse {
    text_html(SPA_INDEX)
}
async fn styles() -> impl IntoResponse {
    static_body(SPA_STYLES.as_bytes(), "text/css")
}
async fn main_js() -> impl IntoResponse {
    static_body(SPA_MAIN_JS.as_bytes(), "text/javascript")
}
async fn api_js() -> impl IntoResponse {
    static_body(SPA_API_JS.as_bytes(), "text/javascript")
}
async fn locale(PathParam(lang): PathParam<String>) -> Response {
    match lang.as_str() {
        "en" => static_body(LOCALE_EN.as_bytes(), "application/json; charset=utf-8"),
        _ => static_body(LOCALE_ZH.as_bytes(), "application/json; charset=utf-8"),
    }
}

fn text_html(body: &'static str) -> Response {
    static_body(body.as_bytes(), "text/html; charset=utf-8")
}

fn static_body(body: &'static [u8], mime: &'static str) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static(mime))],
        body,
    )
        .into_response()
}

// -------- RPC / 能力 / WS --------

async fn rpc_handler(
    State(state): State<Arc<RpcState>>,
    axum::Json(req): axum::Json<rpc::RpcRequest>,
) -> axum::Json<rpc::RpcResponse> {
    axum::Json(rpc::dispatch(&state, &req.cmd, &req.args).await)
}

/// 能力协商（F-2）：向浏览器声明当前可用的原生能力，供前端决定 UI 分支。
async fn capabilities(State(state): State<Arc<RpcState>>) -> axum::Json<serde_json::Value> {
    let _ = state;
    axum::Json(serde_json::json!({
        "ok": true,
        "reveal_available": true,
        "native_pick_available": rpc::NATIVE_PICK.load(std::sync::atomic::Ordering::Relaxed),
    }))
}

/// F6 鉴权中间件：回环 peer 放行；非回环来源校验令牌。
async fn web_auth(
    State(state): State<Arc<RpcState>>,
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::extract::connect_info::ConnectInfo;
    let peer_is_loopback = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().is_loopback())
        .unwrap_or(true);
    if peer_is_loopback || check_token(&state.token, &req) {
        return next.run(req).await;
    }
    (
        axum::http::StatusCode::FORBIDDEN,
        axum::Json(serde_json::json!({
            "ok": false,
            "error": rpc::token_required(&state.token)
        })),
    )
        .into_response()
}

fn check_token(token: &str, req: &axum::http::Request<axum::body::Body>) -> bool {
    if token.is_empty() {
        return true;
    }
    for name in [axum::http::header::AUTHORIZATION.as_str(), "x-universal-token"] {
        if let Some(header) = req.headers().get(name) {
            if let Ok(v) = header.to_str() {
                let v = v.trim();
                if v == token || v.strip_prefix("Bearer ").map(|s| s.trim() == token).unwrap_or(false) {
                    return true;
                }
            }
        }
    }
    if let Some(q) = req.uri().query() {
        for kv in q.split('&') {
            let mut it = kv.splitn(2, '=');
            if it.next() == Some("token") && it.next() == Some(token) {
                return true;
            }
        }
    }
    false
}

/// WebSocket 事件通道：订阅 RpcState.events 广播并转发；兼作 25s 保活。
/// 客户端主动断开或事件通道关闭即结束。
async fn ws_handler(
    State(state): State<Arc<RpcState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |mut socket: WebSocket| async move {
        let mut rx = state.events.subscribe();
        let mut ping = tokio::time::interval(std::time::Duration::from_secs(25));
        loop {
            tokio::select! {
                res = socket.recv() => {
                    match res {
                        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                        // 收到客户端消息回一个 hello 快照（客户端没消息时也能保持连接）
                        Some(Ok(Message::Text(_))) => {
                            if socket.send(Message::Text(r#"{"type":"hello","msg":"ws-ok"}"#.into())).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(_)) => {}
                    }
                }
                msg = rx.recv() => {
                    match msg {
                        Ok(text) => {
                            if socket.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                        // 缓冲区溢出或发送端已关闭：结束该连接
                        Err(_) => break,
                    }
                }
                _ = ping.tick() => {
                    if socket.send(Message::Ping(vec![])).await.is_err() {
                        break;
                    }
                }
            }
        }
    })
}

#[cfg(target_os = "macos")]
fn open_url(url: &str) -> std::io::Result<()> {
    std::process::Command::new("open").arg(url).spawn().map(|_| ())
}

#[cfg(target_os = "linux")]
fn open_url(url: &str) -> std::io::Result<()> {
    std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ())
}

#[cfg(target_os = "windows")]
fn open_url(url: &str) -> std::io::Result<()> {
    std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn().map(|_| ())
}