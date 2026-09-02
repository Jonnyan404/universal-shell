//! 模板验证工具：遍历 templates/ 下模板(+ demo/shell.json)，
//! 对当前 OS/arch 调 GitHub API 检查资产候选能否命中真实 release。
//!
//! 用法：cargo run -p shared --example verify_templates
//! 输出 PASS/FAIL，任一失败 exit 1。可在 CI 中定时运行，监控上游资产命名变化。

use std::collections::BTreeMap;
use std::path::PathBuf;

use shared::config::{ShellConfig, Program};
use shared::github::GitHub;

fn load_all_templates() -> Vec<Program> {
    let mut out = Vec::new();
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../templates"));
    if dir.exists() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(raw) = std::fs::read_to_string(&path) {
                    if let Ok(p) = serde_json::from_str::<Program>(&raw) {
                        out.push(p);
                    } else {
                        println!("!! 模板 JSON 解析失败: {}", path.display());
                    }
                }
            }
        }
    }
    // demo 配置里的程序也纳入
    let demo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../demo/shell.json"));
    if demo.exists() {
        if let Ok(cfg) = ShellConfig::load(&demo) {
            out.extend(cfg.programs);
        }
    }
    out
}

fn main() {
    let github = GitHub::default();
    let arch = std::env::consts::ARCH.to_string();
    let programs = load_all_templates();
    if programs.is_empty() {
        println!("无模板可验证");
        std::process::exit(2);
    }

    if github.token.is_none() {
        println!("!! 未设置 GITHUB_TOKEN，使用匿名请求（60 次/小时限额）。建议：export GITHUB_TOKEN=ghp_xxx");
    }

    println!("== 验证 {} 个程序 (OS={}, arch={}) ==", programs.len(), std::env::consts::OS, arch);
    let mut failed = 0;
    for p in &programs {
        // 跳过演示用假仓库
        if p.id == "demo-echo" {
            println!("[SKIP] {:<12} (演示程序，无真实仓库)", p.id);
            continue;
        }
        let result = (|| -> anyhow::Result<(String, String, Option<String>)> {
            let release = github.latest(&p.repo)?;
            let version = release.tag_name.trim_start_matches('v').to_string();
            let (rule, candidates) = p
                .candidate_names(&arch, &version)
                .ok_or_else(|| anyhow::anyhow!("当前系统无资产规则"))?;
            let (name, _url, digest) = github.match_candidate(&release, &candidates)?;
            Ok((rule.format.clone(), name, digest))
        })();

        match result {
            Ok((format, name, digest)) => {
                let m = p.assets.get(shared::config::os_key()).map(|r| {
                    match r.mode {
                        shared::config::ExtractMode::Whole => "whole",
                        shared::config::ExtractMode::Single => "single",
                        shared::config::ExtractMode::Raw => "raw",
                    }
                }).unwrap_or("?");
                let digest_hint = match &digest {
                    Some(_) => "sha256 ok",
                    None => "no digest!",
                };
                println!("[PASS] {:<12} mode={:<7} format={:<7} {} -> {}", p.id, m, format, digest_hint, name);
                if digest.is_none() {
                    failed += 1;
                    println!("  ^ 上游未提供 digest，C2 校验将退化为无校验");
                }
            }
            Err(e) => {
                println!("[FAIL] {:<12} {}", p.id, e);
                failed += 1;
            }
        }
    }
    println!("== 结果: {}/{} 通过 ==", programs.len() - failed, programs.len());
    if failed > 0 {
        std::process::exit(1);
    }
}

#[allow(dead_code)]
fn _unused() -> BTreeMap<String, String> { BTreeMap::new() }