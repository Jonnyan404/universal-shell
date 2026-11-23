//! 本地看板：`us-svc ui` 起一个只读+可操作的页面。
//!
//! 只绑 127.0.0.1，不常驻也可随时关掉；它是"展示层"，真相仍在配置与调度器里。

use crate::{config::Config, state, Ctx, Service, ServiceState};
use anyhow::{Context, Result};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const INDEX_HTML: &str = include_str!("../ui/index.html");

#[derive(Clone)]
struct App {
    config: PathBuf,
}

pub async fn serve(config: PathBuf, port: u16, open: bool) -> Result<()> {
    let cfg = Config::load(&config)?;
    let ctx = Ctx::new(&config)?;
    let _ = cfg; // 仅用于启动时校验，之后每次请求重新读取
    let _ = ctx;

    let app = Router::new()
        .route("/", get(index))
        .route("/api/services", get(list))
        .route("/api/log", get(log))
        .route("/api/action", post(action))
        .with_state(Arc::new(App {
            config: config.clone(),
        }));

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("绑定 127.0.0.1:{port} 失败"))?;
    let addr = listener.local_addr()?;
    let url = format!("http://{addr}/");
    println!("服务看板: {url}");
    println!("配置文件: {}", config.display());

    if open {
        open_browser(&url);
    }
    axum::serve(listener, app).await?;
    Ok(())
}

fn open_browser(url: &str) {
    let cmd = if cfg!(target_os = "macos") {
        vec!["open", url]
    } else if cfg!(target_os = "windows") {
        vec!["cmd", "/C", "start", "", url]
    } else {
        vec!["xdg-open", url]
    };
    let _ = std::process::Command::new(cmd[0])
        .args(&cmd[1..])
        .spawn();
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

#[derive(Serialize)]
struct ListResp {
    platform: String,
    config: String,
    services: Vec<Row>,
}

#[derive(Serialize)]
struct Row {
    id: String,
    name: String,
    description: String,
    command: String,
    args: Vec<String>,
    scope: String,
    restart: String,
    delay_secs: u64,
    enabled: bool,
    state: ServiceState,
    state_label: String,
    unit: String,
    log: String,
    log_size: u64,
    last_line: String,
}

async fn list(State(app): State<Arc<App>>) -> Result<Json<ListResp>, (StatusCode, String)> {
    let cfg = Config::load(&app.config).map_err(bad)?;
    let ctx = Ctx::new(&app.config).map_err(bad)?;
    let mut rows = Vec::new();
    for s in &cfg.services {
        let st = state(s, &app.config);
        let log = s.log_path(&app.config);
        rows.push(Row {
            id: s.id.clone(),
            name: s.title().to_string(),
            description: s.description.clone(),
            command: s.command.clone(),
            args: s.args.clone(),
            scope: match s.scope {
                crate::Scope::User => "user".into(),
                crate::Scope::System => "system".into(),
            },
            restart: match s.restart {
                crate::Restart::Never => "never".into(),
                crate::Restart::OnFailure => "on_failure".into(),
                crate::Restart::Always => "always".into(),
            },
            delay_secs: s.delay_secs,
            enabled: s.enabled,
            state: st,
            state_label: st.label().into(),
            unit: ctx.unit_path_of(s).display().to_string(),
            log: log.display().to_string(),
            log_size: std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0),
            last_line: last_line(&log).unwrap_or_default(),
        });
    }
    Ok(Json(ListResp {
        platform: format!("{:?}", crate::Platform::current()).to_lowercase(),
        config: app.config.display().to_string(),
        services: rows,
    }))
}

fn bad(e: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, format!("{e:#}"))
}

fn last_line(path: &Path) -> Option<String> {
    let text = read_tail(path, 1)?;
    text.lines().last().map(str::to_string)
}

/// 读文件尾部最多 n 行（不做全量读取：日志可能很大）
fn read_tail(path: &Path, n: usize) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    const CHUNK: u64 = 256 * 1024;
    let start = len.saturating_sub(CHUNK);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = String::new();
    f.take(CHUNK).read_to_string(&mut buf).ok()?;
    let mut lines: Vec<&str> = buf.lines().rev().take(n).collect();
    lines.reverse();
    Some(lines.join("\n"))
}

#[derive(Deserialize)]
struct LogQ {
    id: String,
    #[serde(default = "default_tail")]
    tail: usize,
}

fn default_tail() -> usize {
    300
}

#[derive(Serialize)]
struct LogResp {
    id: String,
    log: String,
    lines: Vec<String>,
    size: u64,
}

async fn log(
    State(app): State<Arc<App>>,
    Query(q): Query<LogQ>,
) -> Result<Json<LogResp>, (StatusCode, String)> {
    let cfg = Config::load(&app.config).map_err(bad)?;
    let s = cfg
        .get(&q.id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("没有 service `{}`", q.id)))?;
    let path = s.log_path(&app.config);
    let text = read_tail(&path, q.tail.min(5000)).unwrap_or_default();
    Ok(Json(LogResp {
        id: s.id.clone(),
        log: path.display().to_string(),
        lines: text.lines().map(str::to_string).collect(),
        size: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
    }))
}

#[derive(Deserialize)]
struct ActionReq {
    id: String,
    action: String,
}

#[derive(Serialize)]
struct ActionResp {
    ok: bool,
    message: String,
}

async fn action(
    State(app): State<Arc<App>>,
    Json(req): Json<ActionReq>,
) -> Json<ActionResp> {
    let cfg = match Config::load(&app.config) {
        Ok(c) => c,
        Err(e) => {
            return Json(ActionResp {
                ok: false,
                message: format!("{e:#}"),
            })
        }
    };
    let Some(s) = cfg.get(&req.id) else {
        return Json(ActionResp {
            ok: false,
            message: format!("没有 service `{}`", req.id),
        });
    };
    let res = match req.action.as_str() {
        "start" => crate::kick(s, false),
        "restart" => crate::kick(s, true),
        "stop" => crate::stop(s),
        "enable" => enable(s, &app.config, &cfg),
        "disable" => crate::uninstall(s, &app.config),
        other => Err(anyhow::anyhow!("未知操作 `{other}`")),
    };
    match res {
        Ok(()) => Json(ActionResp {
            ok: true,
            message: format!("{} 已{}", s.title(), req.action),
        }),
        Err(e) => Json(ActionResp {
            ok: false,
            message: format!("{e:#}"),
        }),
    }
}

fn enable(s: &Service, config: &Path, cfg: &Config) -> Result<()> {
    let ctx = Ctx::new(config)?;
    let items = crate::plan_for(
        &Config {
            services: vec![s.clone()],
        },
        &ctx,
        crate::Platform::current(),
    )?;
    crate::install(&items, cfg)
}

impl Ctx {
    fn unit_path_of(&self, s: &Service) -> PathBuf {
        match crate::Platform::current() {
            crate::Platform::Launchd => crate::launchd::unit_path(s),
            crate::Platform::Systemd => crate::systemd::unit_path(s),
            crate::Platform::Schtasks => crate::schtasks::xml_path(s, &self.config),
        }
    }
}
