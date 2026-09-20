//! 从模板库目录生成静态注册表目录(直接提交进仓库，供 GitHub raw 直连)。
//!
//! 用法：
//!   cargo run -p shared --example generate_registry [<out_dir>] [<templates_dir>]
//!
//! 目录来源优先级：命令行参数 > 环境变量(`US_REGISTRY_OUT_DIR` / `US_TEMPLATES_DIR`) > 默认。
//! 默认 out_dir = `../registry`、templates_dir = `../templates`（均相对 shared crate 目录）。
//!
//! 注：模板库已拆到独立仓库 universal-shell-templates；该仓库 CI 会 checkout 本仓库
//! 作为工具链，因此这里必须支持显式传入两个目录（见其 .github/workflows/update-registry.yml）。
//!
//! 生成：
//!   <out_dir>/manifests.json            轻量索引
//!   <out_dir>/templates/<id>.json       每个模板拷贝一份
//!
//! 说明：此处「清单」由模板的轻量字段(id/name/category/description/repo)实时生成，
//! 保持与模板库单一事实来源一致；模板库里已删除的模板，其生成的快照会被清理。

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args
        .next()
        .map(PathBuf::from)
        .or_else(|| std::env::var("US_REGISTRY_OUT_DIR").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../registry")));
    let templates_dir = args
        .next()
        .map(PathBuf::from)
        .or_else(|| std::env::var("US_TEMPLATES_DIR").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../templates")));

    if !templates_dir.is_dir() {
        eprintln!(
            "模板库目录不存在: {}（用第 2 个参数或 US_TEMPLATES_DIR 指定）",
            templates_dir.display()
        );
        std::process::exit(2);
    }

    let mut texts: Vec<(String, String)> = Vec::new();
    for entry in std::fs::read_dir(&templates_dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }
        let id = entry
            .file_name()
            .to_string_lossy()
            .to_string()
            .replace(".json", "");
        texts.push((id, std::fs::read_to_string(&path).unwrap()));
    }
    // 按 id 排序，保证生成产物跨环境、跨运行确定一致，避免 registry 反复大改
    texts.sort_by(|a, b| a.0.cmp(&b.0));

    let mut indices = Vec::new();
    let mut categories = vec!["file-sharing".to_string(), "sync".to_string(), "proxy".to_string()];
    for (id, content) in &texts {
        let p: shared::config::Program = serde_json::from_str(content).unwrap();
        if !categories.contains(&p.category) {
            categories.push(p.category.clone());
        }
        indices.push(shared::TemplateIndex {
            id: id.clone(),
            name: p.name.clone(),
            category: p.category.clone(),
            description: p.description.clone(),
            repo: p.repo.clone(),
        });
    }

    let manifest = shared::Manifest {
        revision: chrono_now(),
        categories,
        templates: indices,
    };
    let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();

    let tpl_out = out_dir.join("templates");
    std::fs::create_dir_all(&tpl_out).unwrap();
    for (id, content) in &texts {
        std::fs::write(tpl_out.join(format!("{id}.json")), content).unwrap();
    }
    // 清理模板库里已删除的模板快照，避免 registry/ 残留过期模板
    let mut pruned: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&tpl_out).unwrap().flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }
        let id = entry.file_name().to_string_lossy().replace(".json", "");
        if !texts.iter().any(|(kept, _)| *kept == id) {
            if std::fs::remove_file(&path).is_ok() {
                pruned.push(id);
            }
        }
    }
    std::fs::write(out_dir.join("manifests.json"), format!("{manifest_json}\n")).unwrap();

    println!("registry written to {}: {} templates", out_dir.display(), texts.len());
    if !pruned.is_empty() {
        println!("pruned stale snapshots: {pruned:?}");
    }
    println!("templates: {:?}", texts.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>());
}

fn chrono_now() -> String {
    // 简化：用 SystemTime 秒数生成可读 revision
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("rev-{secs}")
}