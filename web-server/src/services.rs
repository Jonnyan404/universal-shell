//! 主界面「服务」页的后端。
//!
//! 只做两件事：把 `<data_dir>/services.toml` 读成界面能渲染的结构，
//! 以及把界面的动作转发给系统调度器。调度与日志的真相仍由 `us-service` 负责。

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use us_service::{config::Config, state, unit_path_of, Ctx, Service};

/// 服务声明文件：与 shell.json 同目录，便于一起备份
pub fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("services.toml")
}

fn ensure_file(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("创建 {} 失败", dir.display()))?;
    }
    std::fs::write(
        path,
        "# us-svc 服务声明（由主界面「服务」页维护，也可手改）\n\
         # 每条服务 = 一条命令 + 参数 + 调度策略，由系统（launchd/systemd/计划任务）负责拉起。\n",
    )
    .with_context(|| format!("初始化 {} 失败", path.display()))?;
    Ok(())
}

fn restart_name(r: us_service::Restart) -> &'static str {
    match r {
        us_service::Restart::Never => "never",
        us_service::Restart::OnFailure => "on_failure",
        us_service::Restart::Always => "always",
    }
}

fn scope_name(s: us_service::Scope) -> &'static str {
    match s {
        us_service::Scope::User => "user",
        us_service::Scope::System => "system",
    }
}

fn platform_name() -> &'static str {
    match us_service::Platform::current() {
        us_service::Platform::Launchd => "launchd",
        us_service::Platform::Systemd => "systemd",
        us_service::Platform::Schtasks => "schtasks",
    }
}

pub fn list(data_dir: &Path) -> Result<Value> {
    let path = config_path(data_dir);
    ensure_file(&path)?;
    let cfg = Config::load(&path)?;
    let mut rows = Vec::new();
    for s in &cfg.services {
        let st = state(s, &path);
        let log = s.log_path(&path);
        rows.push(json!({
            "id": s.id,
            "name": s.title(),
            "description": s.description,
            "command": s.command,
            "args": s.args,
            "env": s.env,
            "cwd": s.cwd.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            "scope": scope_name(s.scope),
            "restart": restart_name(s.restart),
            "delay_secs": s.delay_secs,
            "depends_on": s.depends_on,
            "wait_timeout_secs": s.wait_timeout_secs,
            "ready_port": s.ready_port,
            "ready_file": s.ready_file.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            "enabled": s.enabled,
            "origin": s.origin,
            "state": st,
            "state_label": st.label(),
            "unit": unit_path_of(s, &path).display().to_string(),
            "log": log.display().to_string(),
            "log_size": std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0),
            "last_line": us_service::last_line(&log).unwrap_or_default(),
        }));
    }
    Ok(json!({
        "platform": platform_name(),
        "config": path.display().to_string(),
        "services": rows,
    }))
}

pub fn log(data_dir: &Path, id: &str, tail: usize) -> Result<Value> {
    let path = config_path(data_dir);
    ensure_file(&path)?;
    let cfg = Config::load(&path)?;
    let s = cfg
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("没有 service `{id}`"))?;
    let log_path = s.log_path(&path);
    let text = us_service::read_tail(&log_path, tail.clamp(1, 5000)).unwrap_or_default();
    Ok(json!({
        "id": s.id,
        "log": log_path.display().to_string(),
        "lines": text.lines().collect::<Vec<_>>(),
        "size": std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0),
    }))
}

#[derive(Deserialize, Clone)]
pub struct Draft {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub args: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub log: String,
    #[serde(default)]
    pub delay_secs: u64,
    #[serde(default)]
    pub depends_on: String,
    #[serde(default)]
    pub wait_timeout_secs: u64,
    #[serde(default)]
    pub ready_port: String,
    #[serde(default)]
    pub ready_file: String,
    #[serde(default)]
    pub restart: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub apply: bool,
}

fn default_true() -> bool {
    true
}

/// 把 `services.toml` 里的某一条装进系统调度器。
///
/// 重新装载会先卸载再装载 launchd 作业 / systemd 单元 / 计划任务，
/// 也就是会**重启**该服务 —— 这正是「改了参数要生效」需要的行为。
pub fn apply_one(dir: &Path, id: &str) -> anyhow::Result<()> {
    let path = config_path(dir);
    let cfg = us_service::config::Config::load(&path)?;
    let svc = cfg
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("没有 service `{id}`"))?;
    let one = us_service::Config {
        services: vec![svc.clone()],
    };
    let ctx = us_service::Ctx::new(&path)?;
    let items = us_service::plan_for(&one, &ctx, us_service::Platform::current())?;
    us_service::install(&items, &cfg)
}

pub fn save(data_dir: &Path, d: Draft) -> Result<Value> {
    let path = config_path(data_dir);
    ensure_file(&path)?;
    let mut cfg = Config::load(&path)?;

    let cwd = d.cwd.trim();
    let log = d.log.trim();
    let svc = Service {
        id: d.id.trim().to_string(),
        name: d.name.trim().to_string(),
        description: d.description.trim().to_string(),
        command: d.command.trim().to_string(),
        args: us_service::parse_args(&d.args),
        env: us_service::parse_env(&d.env)?,
        cwd: (!cwd.is_empty()).then(|| PathBuf::from(cwd)),
        log: (!log.is_empty()).then(|| PathBuf::from(log)),
        delay_secs: d.delay_secs,
        depends_on: us_service::parse_ids(&d.depends_on),
        wait_timeout_secs: if d.wait_timeout_secs == 0 { 60 } else { d.wait_timeout_secs },
        ready_port: d.ready_port.trim().parse().ok(),
        ready_file: (!d.ready_file.trim().is_empty())
            .then(|| PathBuf::from(d.ready_file.trim())),
        restart: us_service::restart_from(&d.restart),
        restart_secs: 10,
        scope: us_service::scope_from(&d.scope),
        enabled: d.enabled,
        // 手工创建/编辑：不标记来源（派生条目由 shared 同步写入）
        origin: None,
    };
    let id = svc.id.clone();
    cfg.upsert(svc.clone())?;
    cfg.save(&path)?;

    let mut message = format!("{id} 已保存");
    if d.apply && svc.enabled {
        let ctx = Ctx::new(&path)?;
        let items = us_service::plan_for(
            &Config {
                services: vec![svc.clone()],
            },
            &ctx,
            us_service::Platform::current(),
        )?;
        match us_service::install(&items, &cfg) {
            Ok(()) => message = format!("{id} 已保存并启用"),
            // 配置已经写好了，装载失败只作为提示返回，不让界面以为没保存
            Err(e) => message = format!("{id} 已保存，但装载失败：{e:#}"),
        }
    }
    Ok(json!({ "ok": true, "message": message }))
}

pub fn remove(data_dir: &Path, id: &str, uninstall: bool) -> Result<Value> {
    let path = config_path(data_dir);
    ensure_file(&path)?;
    let mut cfg = Config::load(&path)?;
    let svc = cfg
        .get(id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("没有 service `{id}`"))?;
    if uninstall {
        let _ = us_service::uninstall(&svc, &path);
    }
    cfg.remove(id);
    cfg.save(&path)?;
    Ok(json!({ "ok": true, "message": format!("{id} 已删除") }))
}

pub fn action(data_dir: &Path, id: &str, action: &str) -> Result<Value> {
    let path = config_path(data_dir);
    ensure_file(&path)?;
    let cfg = Config::load(&path)?;
    let svc = cfg
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("没有 service `{id}`"))?;
    let message = match action {
        "start" => {
            us_service::kick(svc, false)?;
            format!("{} 已启动", svc.title())
        }
        "restart" => {
            us_service::kick(svc, true)?;
            format!("{} 已重启", svc.title())
        }
        "stop" => {
            us_service::stop(svc)?;
            format!("{} 已停止", svc.title())
        }
        "enable" => {
            let ctx = Ctx::new(&path)?;
            let items = us_service::plan_for(
                &Config {
                    services: vec![svc.clone()],
                },
                &ctx,
                us_service::Platform::current(),
            )?;
            us_service::install(&items, &cfg)?;
            format!("{} 已启用自启", svc.title())
        }
        "disable" => {
            us_service::uninstall(svc, &path)?;
            format!("{} 已取消自启", svc.title())
        }
        other => anyhow::bail!("未知操作 `{other}`"),
    };
    Ok(json!({ "ok": true, "message": message }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("us-rpc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn list_creates_the_file_and_returns_empty() {
        let dir = tmp("list");
        let v = list(&dir).unwrap();
        assert_eq!(v["services"].as_array().unwrap().len(), 0);
        assert!(config_path(&dir).exists(), "首次读取应初始化文件");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_list_then_remove_roundtrip() {
        let dir = tmp("save");
        let msg = save(
            &dir,
            Draft {
                id: "demo".into(),
                name: "演示".into(),
                description: String::new(),
                command: "/bin/echo".into(),
                args: "hello\nworld".into(),
                env: "A=1\nB=2".into(),
                cwd: String::new(),
                log: String::new(),
                delay_secs: 3,
                depends_on: String::new(),
                wait_timeout_secs: 0,
                ready_port: String::new(),
                ready_file: String::new(),
                restart: "on_failure".into(),
                scope: "user".into(),
                enabled: true,
                apply: false,
            },
        )
        .unwrap();
        assert!(msg["ok"].as_bool().unwrap());
        assert!(msg["message"].as_str().unwrap().contains("已保存"));

        let v = list(&dir).unwrap();
        let one = &v["services"][0];
        assert_eq!(one["id"], "demo");
        assert_eq!(one["args"].as_array().unwrap().len(), 2, "逐行参数");
        assert_eq!(one["env"]["A"], "1");
        assert_eq!(one["delay_secs"], 3);
        assert_eq!(one["restart"], "on_failure");

        remove(&dir, "demo", false).unwrap();
        assert_eq!(list(&dir).unwrap()["services"].as_array().unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_rejects_bad_env_and_bad_id() {
        let dir = tmp("bad");
        let mut d = Draft {
            id: "ok".into(),
            name: String::new(),
            description: String::new(),
            command: "/bin/true".into(),
            args: String::new(),
            env: "NOT_A_PAIR".into(),
            cwd: String::new(),
            log: String::new(),
            delay_secs: 0,
            depends_on: String::new(),
            wait_timeout_secs: 0,
            ready_port: String::new(),
            ready_file: String::new(),
            restart: "never".into(),
            scope: "user".into(),
            enabled: true,
            apply: false,
        };
        assert!(save(&dir, d.clone()).is_err(), "env 非法应报错");

        d.env = String::new();
        d.id = "../escape".into();
        assert!(save(&dir, d).is_err(), "id 含路径应报错");
        let _ = std::fs::remove_dir_all(&dir);
    }

}
