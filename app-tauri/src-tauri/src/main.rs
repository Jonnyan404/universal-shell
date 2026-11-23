// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 发布版不再分发独立的 us-svc，调度器的包装器就指向本程序：
    // `app-tauri run --config … --id …` 只跑目标命令，不初始化窗口。
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("run") {
        match us_service::run_cli(&argv[1..]) {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                eprintln!("app-tauri run: {e:#}");
                std::process::exit(1);
            }
        }
    }
    app_tauri_lib::run()
}
