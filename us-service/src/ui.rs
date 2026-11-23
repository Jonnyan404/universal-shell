//! 本地看板：`us-svc ui` 起一个只读+可操作的页面。
//!
//! 只绑 127.0.0.1，不常驻也可随时关掉；它是"展示层"，真相仍在配置与调度器里。

use crate::config::Config;
use crate::{state, Ctx, Service, ServiceState};
use std::collections::BTreeMap;
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
        .route("/api/save", post(save))
        .route("/api/remove", post(remove))
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
    /// 编辑表单要用：不返回这些字段，界面保存时会把它们清空
    env: BTreeMap<String, String>,
    cwd: String,
    scope: String,
    restart: String,
    delay_secs: u64,
    depends_on: Vec<String>,
    wait_timeout_secs: u64,
    ready_port: Option<u16>,
    ready_file: String,
    enabled: bool,
    origin: Option<String>,
    state: ServiceState,
    state_label: String,
    unit: String,
    log: String,
    log_size: u64,
    last_line: String,
}

async fn list(State(app): State<Arc<App>>) -> Result<Json<ListResp>, (StatusCode, String)> {
    let cfg = Config::load(&app.config).map_err(bad)?;
    // 只为尽早校验配置可读；路径计算改用 crate::unit_path_of
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
            env: s.env.clone(),
            cwd: s.cwd.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
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
            depends_on: s.depends_on.clone(),
            wait_timeout_secs: s.wait_timeout_secs,
            ready_port: s.ready_port,
            ready_file: s
                .ready_file
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            enabled: s.enabled,
            origin: s.origin.clone(),
            state: st,
            state_label: st.label().into(),
            unit: crate::unit_path_of(s, &app.config).display().to_string(),
            log: log.display().to_string(),
            log_size: std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0),
            last_line: crate::last_line(&log).unwrap_or_default(),
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
    let text = crate::read_tail(&path, q.tail.min(5000)).unwrap_or_default();
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


#[derive(Deserialize)]
struct SaveReq {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    command: String,
    /// 每行一个参数（保留行内空格）
    #[serde(default)]
    args: String,
    /// 每行 `KEY=VALUE`
    #[serde(default)]
    env: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    log: String,
    #[serde(default)]
    delay_secs: u64,
    /// 依赖的服务 id（逗号/换行分隔）
    #[serde(default)]
    depends_on: String,
    #[serde(default)]
    wait_timeout_secs: u64,
    /// 就绪判定：端口 / 文件（留空表示用「进程在跑」）
    #[serde(default)]
    ready_port: String,
    #[serde(default)]
    ready_file: String,
    #[serde(default)]
    restart: String,
    #[serde(default)]
    scope: String,
    #[serde(default = "default_true_ui")]
    enabled: bool,
    /// 保存后立刻装载到系统
    #[serde(default)]
    apply: bool,
}

fn default_true_ui() -> bool {
    true
}

async fn save(State(app): State<Arc<App>>, Json(req): Json<SaveReq>) -> Json<ActionResp> {
    let result = (|| -> anyhow::Result<String> {
        let mut cfg = Config::load(&app.config)?;
        let env = crate::parse_env(&req.env)?;
        let svc = Service {
            id: req.id.trim().to_string(),
            name: req.name.trim().to_string(),
            description: req.description.trim().to_string(),
            command: req.command.trim().to_string(),
            args: crate::parse_args(&req.args),
            env,
            cwd: non_empty(&req.cwd).map(std::path::PathBuf::from),
            log: non_empty(&req.log).map(std::path::PathBuf::from),
            delay_secs: req.delay_secs,
            depends_on: crate::parse_ids(&req.depends_on),
            wait_timeout_secs: if req.wait_timeout_secs == 0 { 60 } else { req.wait_timeout_secs },
            ready_port: req.ready_port.trim().parse().ok(),
            ready_file: (!req.ready_file.trim().is_empty())
                .then(|| std::path::PathBuf::from(req.ready_file.trim())),
            restart: crate::restart_from(&req.restart),
            restart_secs: 10,
            scope: crate::scope_from(&req.scope),
            enabled: req.enabled,
            // 界面新建的服务不是从程序派生的
            origin: None,
        };
        let id = svc.id.clone();
        cfg.upsert(svc.clone())?;
        cfg.save(&app.config)?;

        if req.apply && svc.enabled {
            let ctx = Ctx::new(&app.config)?;
            let items = crate::plan_for(
                &Config {
                    services: vec![svc.clone()],
                },
                &ctx,
                crate::Platform::current(),
            )?;
            crate::install(&items, &cfg)
                .map_err(|e| anyhow::anyhow!("已保存，但装载失败：{e:#}"))?;
            return Ok(format!("{id} 已保存并启用"));
        }
        Ok(format!("{id} 已保存"))
    })();

    match result {
        Ok(message) => Json(ActionResp { ok: true, message }),
        Err(e) => Json(ActionResp {
            ok: false,
            message: format!("{e:#}"),
        }),
    }
}

fn non_empty(s: &str) -> Option<&str> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

#[derive(Deserialize)]
struct RemoveReq {
    id: String,
    /// 一并从系统卸载
    #[serde(default)]
    uninstall: bool,
}

async fn remove(State(app): State<Arc<App>>, Json(req): Json<RemoveReq>) -> Json<ActionResp> {
    let result = (|| -> anyhow::Result<String> {
        let mut cfg = Config::load(&app.config)?;
        let svc = cfg.get(&req.id).cloned();
        if svc.is_none() {
            anyhow::bail!("没有 service `{}`", req.id);
        }
        if req.uninstall {
            if let Some(s) = &svc {
                let _ = crate::uninstall(s, &app.config);
            }
        }
        if !cfg.remove(&req.id) {
            anyhow::bail!("没有 service `{}`", req.id);
        }
        cfg.save(&app.config)?;
        Ok(format!("{} 已删除", req.id))
    })();
    match result {
        Ok(message) => Json(ActionResp { ok: true, message }),
        Err(e) => Json(ActionResp {
            ok: false,
            message: format!("{e:#}"),
        }),
    }
}
