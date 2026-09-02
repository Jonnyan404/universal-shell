//! 从本地 templates/*.json 生成静态注册表目录(直接提交进仓库，供 GitHub raw 直连)。
//!
//! 用法：cargo run -p shared --example generate_registry [<out_dir>]
//! 默认输出到 registry/（git 版本控制，raw 链接可用）。
//! 生成：
//!   <out_dir>/manifests.json            手动的轻量索引
//!   <out_dir>/templates/<id>.json       每个模板拷贝一份
//!
//! 说明：此处「清单」由模板的轻量字段(id/name/category/description/repo)实时生成，
//! 保持与本地模板库单一事实来源一致。

use std::path::PathBuf;

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../registry"))
        });
    let templates_dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../templates"));

    let mut texts: Vec<(String, String)> = Vec::new();
    for entry in std::fs::read_dir(&templates_dir).unwrap().flatten() {
        let id = entry
            .file_name()
            .to_string_lossy()
            .to_string()
            .replace(".json", "");
        texts.push((id, std::fs::read_to_string(entry.path()).unwrap()));
    }

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
    std::fs::write(out_dir.join("manifests.json"), format!("{manifest_json}\n")).unwrap();

    println!("registry written to {}: {} templates", out_dir.display(), texts.len());
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