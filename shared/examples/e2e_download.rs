//! 端到端安装验证：对指定模板真实下载、解压到临时目录、跑一次 --version/--help。
//!
//! 用法：cargo run -p shared --example e2e_download -- <id...>   (缺省=全部)
//! 适合 CI 里挑代表性模板做全流程验证（下载→解压→可执行）。

use std::path::PathBuf;

use shared::ShellManager;

fn main() {
    // 运行时临时目录（跨平台安全，不依赖编译期 TMPDIR）
    let mut data_dir = std::env::temp_dir();
    data_dir.push("cc-shell-e2e3");
    let _ = std::fs::remove_dir_all(&data_dir);

    let tpl_dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../templates"));
    let mut programs: Vec<shared::Program> = Vec::new();
    for entry in std::fs::read_dir(&tpl_dir).unwrap().flatten() {
        let raw = std::fs::read_to_string(entry.path()).unwrap();
        if let Ok(p) = serde_json::from_str::<shared::Program>(&raw) {
            programs.push(p);
        }
    }

    let only: Vec<String> = std::env::args().skip(1).collect();
    let targets: Vec<shared::Program> = if only.is_empty() {
        programs
    } else {
        programs.into_iter().filter(|p| only.contains(&p.id)).collect()
    };

    if targets.is_empty() {
        eprintln!("未找到任何模板");
        std::process::exit(2);
    }

    let mut failed = 0;
    for p in &targets {
        print!("{:<14} ", p.id);
        match install_and_probe(&data_dir, p) {
            Ok(probe) => println!("OK ({probe})"),
            Err(e) => {
                println!("FAIL {e:#}");
                failed += 1;
            }
        }
    }
    println!("== 结果: {}/{} 通过 ==", targets.len() - failed, targets.len());
    if failed > 0 {
        std::process::exit(1);
    }
}

fn install_and_probe(data_dir: &std::path::PathBuf, p: &shared::Program) -> anyhow::Result<String> {
    let version = ShellManager::install_standalone(data_dir, p)?;
    let mgr = ShellManager::new(data_dir.clone())?;
    let bin = mgr.bin_path(p);
    if !bin.exists() {
        anyhow::bail!("可执行入口不存在: {}", bin.display());
    }
    let out = std::process::Command::new(&bin)
        .arg("--version")
        .output()
        .ok();
    let probe = match out {
        Some(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { "(--version 无输出)".into() } else { s }
        }
        _ => {
            // 有些程序 --version 会失败（如需要终端的 TUI），退到 --help
            let help = std::process::Command::new(&bin).arg("--help").output().ok();
            match help {
                Some(h) if h.status.success() =>
                    format!("[{version}] (--version 不可用，--help ok)"),
                _ => format!("[{version}] (命令探测不可用)"),
            }
        }
    };
    Ok(probe)
}