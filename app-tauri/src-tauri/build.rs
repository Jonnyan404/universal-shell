//! 生成前端 locale JSON：以 `shared/locales/*.yml` 为单一翻译源，
//! 在构建时转换为 `app-tauri/src/locales/*.json` 供前端 `fetch` 使用。

use serde_yaml::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    // 路径相对本包目录（app-tauri/src-tauri）计算
    let locales_dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../shared/locales");
    let out_dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/locales");
    fs::create_dir_all(&out_dir).expect("create app-tauri/src/locales");

    let mut any_file_rerun = false;
    if let Ok(rd) = fs::read_dir(&locales_dir) {
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "yml") {
                continue;
            }
            println!("cargo:rerun-if-changed={}", path.display());
            any_file_rerun = true;
            let text = fs::read_to_string(&path).expect("read yml");
            let value: Value = serde_yaml::from_str(&text).expect("parse yml");
            let obj = match value {
                Value::Mapping(m) => m,
                _ => panic!("locale yml root must be a mapping"),
            };
            // 扁平 key -> 字符串
            let flat: serde_json::Map<String, serde_json::Value> = obj
                .into_iter()
                .filter_map(|(k, v)| match (k, v) {
                    (Value::String(k), Value::String(v)) => Some((k, serde_json::Value::String(v))),
                    _ => None,
                })
                .collect();
            let file_stem = path
                .file_stem()
                .expect("filename")
                .to_string_lossy()
                .to_string();
            let json_path = out_dir.join(format!("{file_stem}.json"));
            let pretty = serde_json::to_string_pretty(&flat).expect("serialize json");
            fs::write(&json_path, pretty).expect("write locale json");
            println!("cargo:rerun-if-changed={}", json_path.display());
        }
    }
    if !any_file_rerun {
        println!("cargo:rerun-if-changed={}", locales_dir.display());
    }
    tauri_build::build()
}
