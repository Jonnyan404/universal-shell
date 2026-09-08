//! 独立运行 web 服务用于 e2e 手测：
//! `cargo run -p web-server --example dev` —— 用真实数据目录起服务并打印 URL。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use shared::ShellManager;
use web_server::WebServerHandle;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let data_dir = dirs::data_dir()
        .map(|d| d.join("universal-shell"))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let config_path = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| data_dir.join("shell.json"));

    let mut manager = ShellManager::new(data_dir.clone()).expect("init data dir");
    if config_path.exists() {
        manager.load_config(&config_path).expect("load config");
    } else if let Ok(cwd) = std::env::current_dir() {
        let cand = cwd.join("shell.json");
        if cand.exists() {
            manager.load_config(&cand).unwrap();
        }
    }

    let manager = Arc::new(Mutex::new(manager));
    log::info!("data_dir={}", data_dir.display());

    let handle = start_serve(manager, config_path)?;
    println!("Web 管理界面已启动：{}", handle.url);
    println!("按 Ctrl-C 退出。");
    let _ = handle;
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn start_serve(manager: Arc<Mutex<ShellManager>>, config_path: std::path::PathBuf) -> anyhow::Result<WebServerHandle> {
    web_server::start(manager, config_path, "127.0.0.1", 0)
}