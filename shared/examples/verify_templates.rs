//! 模板验证工具：遍历 templates/ 与第三方源示例 templates/ 下模板，
//! 对当前 OS/arch 检查：
//! - GitHub 模板：调 GitHub API 检查资产候选能否命中真实 release；
//! - HTTP 源模板(source.kind=http)：探版本 + 直链可用性 + sha256 可解析。
//!
//! 用法：cargo run -p shared --example verify_templates
//! 输出 PASS/FAIL，任一失败 exit 1。可在 CI 中定时运行，监控上游资产命名变化。

use std::path::PathBuf;

use shared::config::Program;
use shared::github::GitHub;

fn load_all_templates() -> Vec<Program> {
    let mut out = Vec::new();
    let dirs = [
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../templates")),
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../registry/example-third-party/templates"
        )),
    ];
    for dir in dirs {
        if !dir.exists() {
            continue;
        }
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
    out
}

fn main() {
    let arch = std::env::consts::ARCH.to_string();
    let programs = load_all_templates();
    if programs.is_empty() {
        println!("无模板可验证");
        std::process::exit(2);
    }

    println!(
        "== 验证 {} 个程序 (OS={}, arch={}) ==",
        programs.len(),
        std::env::consts::OS,
        arch
    );
    let mut failed = 0;
    if GitHub::default().token.is_none() {
        println!("!! 未设置 GITHUB_TOKEN，GitHub 模板使用匿名请求（60 次/小时限额）。建议：export GITHUB_TOKEN=ghp_xxx");
    }
    for p in &programs {
        let result = if p.source.as_ref().map_or(false, |s| s.is_http()) {
            verify_http_template(p, &arch)
        } else {
            verify_github_template(p, &arch)
        };

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
                    println!("  ^ 未提供 digest，C2 校验将退化为无校验");
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

/// GitHub 模板：走 release API 匹配候选资产。
fn verify_github_template(p: &Program, arch: &str) -> anyhow::Result<(String, String, Option<String>)> {
    let github = GitHub::default();
    let release = github.latest(&p.repo)?;
    let version = release.tag_name.trim_start_matches('v').to_string();
    let (rule, candidates) = p
        .candidate_names(arch, &version)
        .ok_or_else(|| anyhow::anyhow!("当前系统无资产规则"))?;
    let (name, _url, digest) = github.match_candidate(&release, &candidates)?;
    Ok((rule.format.clone(), name, digest))
}

/// HTTP 源模板：探版本 + 直链可达 + sha256 可解析（不静默降级）。
fn verify_http_template(p: &Program, arch: &str) -> anyhow::Result<(String, String, Option<String>)> {
    let src = p.source.as_ref().unwrap();
    let hs = shared::source_http::HttpSource::default();
    let version = hs.latest_version(src)?;
    let (rule, urls) = hs.asset_urls(p, &version, arch)?;

    // 至少一个直链可达(跟随重定向，不读 body)
    let mut reachable: Option<String> = None;
    for url in &urls {
        let resp = hs.client.get(url).send()?;
        if resp.status().is_success() {
            reachable = Some(url.clone());
            break;
        }
    }
    let url = reachable.ok_or_else(|| anyhow::anyhow!("直链全部不可达: {urls:?}"))?;

    // 配置了 sha256_url 就必须能解析出摘要(否则算 FAIL，防止静默降级)
    let digest = match hs.declared_sha256(p, src, &version, arch, &rule)? {
        Some(hex) => Some(hex),
        None if src.sha256_url.trim().is_empty() => None,
        None => return Err(anyhow::anyhow!("sha256_url 未解析出摘要: {}", src.sha256_url)),
    };
    let name = shared::source_http::HttpSource::filename_from(&url);
    Ok((rule.format.clone(), name, digest))
}