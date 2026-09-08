//! 无窗口 CLI（F-3/F13）：只起本地 Web 管理服务，不弹 UI——
//! 给树莓派 / NAS / 无桌面环境远程管理的场景：
//!
//! ```sh
//! cargo run -p app-cli --release
//! us-web --config /path/shell.json
//! us-web --bind 0.0.0.0 --port 45990
//! ```
//!
//! 默认读取配置里的 bind/port（端口 0 = 自动高位随机）；命令行参数优先。
//! 非 loopback 绑定会打印 `?token=` 访问链接（F6 鉴权）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use shared::ShellManager;

static STOP: AtomicBool = AtomicBool::new(false);

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut config_arg: Option<PathBuf> = None;
    let mut bind_arg: Option<String> = None;
    let mut port_arg: Option<u16> = None;
    let mut cursor = std::env::args().skip(1);
    while let Some(a) = cursor.next() {
        match a.as_str() {
            "--config" | "-c" => config_arg = cursor.next().map(PathBuf::from),
            "--bind" => bind_arg = cursor.next(),
            "--port" | "-p" => port_arg = cursor.next().and_then(|s| s.parse().ok()),
            "--help" | "-h" => {
                println!(
                    "us-web — Universal Shell headless server\n\
                     \n\
                     Usage: us-web [--config <path>] [--bind <ip>] [--port <n>]\n\
                     \n\
                     Defaults come from shell.json `web` settings\n\
                     (loopback bind, auto port). Ctrl-C to stop."
                );
                return Ok(());
            }
            other => {
                anyhow::bail!("unknown argument: {other} (see --help)");
            }
        }
    }

    let data_dir = dirs::data_dir()
        .map(|d| d.join("universal-shell"))
        .unwrap_or_else(|| PathBuf::from("."));
    let config_path = config_arg.unwrap_or_else(|| data_dir.join("shell.json"));

    let mut manager = ShellManager::new(data_dir.clone()).expect("init data dir");
    if config_path.exists() {
        manager.load_config(&config_path).expect("load config");
    }
    log::info!("data_dir={}", data_dir.display());
    log::info!("config={}", config_path.display());

    let (bind, port) = {
        let w = manager.web_settings();
        (bind_arg.unwrap_or_else(|| w.effective_bind().to_string()), port_arg.unwrap_or(w.port))
    };

    let manager = Arc::new(Mutex::new(manager));
    let mut handle = web_server::start_preferred(manager.clone(), config_path, &bind, port)?;

    let token = {
        let mgr = manager.lock().unwrap();
        mgr.web_settings().token.clone()
    };
    print_url(&handle, &bind, &token);
    println!("Web 管理服务已启动，Ctrl-C 退出。");

    ctrlc::set_handler(|| STOP.store(true, Ordering::SeqCst)).expect("set ctrlc handler");
    while !STOP.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    println!("stopping…");
    handle.stop();
    Ok(())
}

fn print_url(handle: &web_server::WebServerHandle, bind: &str, token: &str) {
    let loopback = bind.is_empty() || bind == "127.0.0.1" || bind == "::1" || bind == "localhost";
    if loopback {
        println!("loopback: {}", handle.url);
        return;
    }
    // 非回环：打印端口 + token，用户按需换成本机局域网 IP
    println!("bind:     {bind}");
    println!("port:     {}", handle.port);
    if !token.is_empty() {
        println!("token:    {token}");
        println!("访问示例: http://<本机IP>:{}/?token={token}", handle.port);
    } else {
        println!("访问示例: http://<本机IP>:{}/", handle.port);
    }
}