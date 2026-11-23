//! us-svc 命令行：声明式服务的生成 / 装载 / 运行 / 查看。

use std::path::{Path, PathBuf};
use us_service::config::Config;
use us_service::{plan, plan_for, probe, runner, Ctx, Platform};

fn main() {
    if let Err(e) = dispatch() {
        eprintln!("us-svc: {e:#}");
        std::process::exit(1);
    }
}

fn dispatch() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        usage();
        return Ok(());
    };
    let rest = &args[1..];
    match cmd {
        "gen" => cmd_gen(rest),
        "apply" => cmd_apply(rest),
        "run" => cmd_run(rest),
        "list" => cmd_list(rest),
        "status" => cmd_status(rest),
        "disable" => cmd_disable(rest),
        "ui" => cmd_ui(rest),
        "-h" | "--help" | "help" => {
            usage();
            Ok(())
        }
        other => anyhow::bail!("未知子命令 `{other}`（用 --help 看用法）"),
    }
}

fn usage() {
    println!(
        "\
us-svc —— 声明式本地服务调度

用法:
  us-svc gen    <services.toml> [--out DIR] [--platform launchd|systemd|schtasks] [--wrapper PATH]
                只生成配置，不碰系统（方便先看产物 / 给别的机器用）
  us-svc apply  <services.toml> [--dry-run]
                写入单元文件并让调度器接管（system 级需自行 sudo）
  us-svc run    --config <services.toml> --id <service-id>
                调度器调用的入口：注入 env、重定向日志、透传退出码
  us-svc list   <services.toml>
  us-svc status <services.toml>
  us-svc disable <services.toml> --id <service-id>
                停止并移除该服务的系统配置（配置本身不动）
  us-svc ui     <services.toml> [--port 45980] [--open]
                本地看板：状态 / 启停 / 日志 tail"

    );
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// 位置参数：第一个既不是 --flag 也不是 flag 值的参数
fn positional(args: &[String]) -> Option<&str> {
    args.iter()
        .enumerate()
        .find(|(i, a)| {
            if a.starts_with('-') {
                return false;
            }
            // 若它是某个 --flag 的值就跳过
            !(*i > 0 && args[i - 1].starts_with("--"))
        })
        .map(|(_, a)| a.as_str())
}

fn cmd_gen(args: &[String]) -> anyhow::Result<()> {
    let path = require_config(args)?;
    let cfg = Config::load(&path)?;
    // --wrapper 用于「给别的机器生成」：默认取当前 us-svc 自身
    let ctx = match flag(args, "--wrapper") {
        Some(w) => Ctx::with_wrapper(&path, w),
        None => Ctx::new(&path)?,
    };
    let platform = match flag(args, "--platform") {
        Some("launchd") => Platform::Launchd,
        Some("systemd") => Platform::Systemd,
        Some("schtasks") => Platform::Schtasks,
        Some(other) => anyhow::bail!("未知 --platform `{other}`"),
        None => Platform::current(),
    };
    let items = plan_for(&cfg, &ctx, platform)?;
    if items.is_empty() {
        println!("没有 enabled 的服务");
        return Ok(());
    }
    match flag(args, "--out") {
        Some(dir) => {
            let written = us_service::write_plans(&items, Path::new(dir))?;
            for p in written {
                println!("{}", p.display());
            }
        }
        None => {
            for it in &items {
                println!("========== {} -> {} ==========", it.service_id, it.path.display());
                print!("{}", it.content);
            }
        }
    }
    Ok(())
}

fn cmd_apply(args: &[String]) -> anyhow::Result<()> {
    let path = require_config(args)?;
    let cfg = Config::load(&path)?;
    let ctx = Ctx::new(&path)?;
    let items = plan(&cfg, &ctx)?;
    if has(args, "--dry-run") {
        for it in &items {
            println!("would install {} -> {}", it.service_id, it.path.display());
            if let Some(s) = cfg.get(&it.service_id) {
                println!("  log: {}", s.log_path(&path).display());
            }
        }
        return Ok(());
    }
    us_service::install(&items, &cfg)?;
    for it in &items {
        println!("installed {}", it.service_id);
        if let Some(s) = cfg.get(&it.service_id) {
            println!("  unit: {}", it.path.display());
            println!("  log:  {}", s.log_path(&path).display());
        }
    }
    Ok(())
}

fn cmd_run(args: &[String]) -> anyhow::Result<()> {
    let config = flag(args, "--config")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("缺少 --config"))?;
    let id = flag(args, "--id").ok_or_else(|| anyhow::anyhow!("缺少 --id"))?;
    let cfg = Config::load(&config)?;
    let svc = cfg
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("配置里没有 service `{id}`"))?;
    let log = svc.log_path(&config);
    let code = runner::run(svc, &log)?;
    std::process::exit(code);
}

fn cmd_list(args: &[String]) -> anyhow::Result<()> {
    let path = require_config(args)?;
    let cfg = Config::load(&path)?;
    println!(
        "{:<16} {:<8} {:<10} {:<6} {}",
        "id", "scope", "restart", "delay", "command"
    );
    for s in &cfg.services {
        println!(
            "{:<16} {:<8} {:<10} {:<6} {} {}",
            s.id,
            format!("{:?}", s.scope).to_lowercase(),
            format!("{:?}", s.restart).to_lowercase(),
            s.delay_secs,
            s.command,
            if s.enabled { "" } else { "(disabled)" }
        );
    }
    Ok(())
}

fn cmd_status(args: &[String]) -> anyhow::Result<()> {
    let path = require_config(args)?;
    let cfg = Config::load(&path)?;
    let ctx = Ctx::new(&path)?;
    let items = plan(&cfg, &ctx)?;
    for it in &items {
        let Some(s) = cfg.get(&it.service_id) else {
            continue;
        };
        let exists = if it.path.exists() { "已写入" } else { "未写入" };
        println!(
            "{:<16} {:<8} {} / {}",
            s.id,
            probe(s, &path),
            exists,
            it.path.display()
        );
    }
    Ok(())
}

fn cmd_disable(args: &[String]) -> anyhow::Result<()> {
    let path = require_config(args)?;
    let id = flag(args, "--id").ok_or_else(|| anyhow::anyhow!("缺少 --id"))?;
    let cfg = Config::load(&path)?;
    let svc = cfg
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("配置里没有 service `{id}`"))?;
    us_service::uninstall(svc, &path)?;
    println!("已移除 {id} 的系统配置（services.toml 未改动）");
    Ok(())
}

fn cmd_ui(args: &[String]) -> anyhow::Result<()> {
    let path = require_config(args)?;
    Config::load(&path)?;
    let port: u16 = match flag(args, "--port") {
        Some(v) => v.parse()?,
        None => 45980,
    };
    let open = has(args, "--open");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(us_service::ui::serve(path, port, open))
}

fn require_config(args: &[String]) -> anyhow::Result<PathBuf> {
    positional(args)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("缺少配置文件路径（用法见 --help）"))
}
