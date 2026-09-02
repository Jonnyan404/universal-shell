//! 注册表客户端 example：起一个本地 http.server 托管静态注册表，验证
//! 清单拉取 / 模板惰性拉取 / 304 缓存 / 离线回退 / 签名验签(C6)。
//!
//! 前置：先运行 `cargo run -p shared --example generate_registry` 生成，
//! 且已由 `sign_registry` 生成 manifests.sig（仓库中已提交）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

fn main() {
    let repo_registry = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../registry"));
    let serve = PathBuf::from("/var/folders/r0/gbm1g1gs3gg1tkd3_8gcz5380000gn/T/opencode/registry-serve");
    let cache = PathBuf::from("/var/folders/r0/gbm1g1gs3gg1tkd3_8gcz5380000gn/T/opencode/registry-cache");
    let _ = std::fs::remove_dir_all(&cache);
    let _ = std::fs::remove_dir_all(&serve);
    // 把仓库 registry 复制到临时服务目录
    let _ = std::fs::create_dir_all(&serve.join("templates"));
    for entry in std::fs::read_dir(&repo_registry).unwrap().flatten() {
        let path = entry.path();
        let dest = serve.join(entry.file_name());
        if path.is_dir() {
            let d = serve.join(entry.file_name());
            std::fs::create_dir_all(&d).unwrap();
            for f in std::fs::read_dir(&path).unwrap().flatten() {
                std::fs::copy(f.path(), d.join(f.file_name())).unwrap();
            }
        } else {
            std::fs::copy(&path, dest).unwrap();
        }
    }

    // 起本地静态服务器
    let mut server = Command::new("python3")
        .args(["-m", "http.server", "8123", "--directory"])
        .arg(&serve)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start server");
    thread::sleep(Duration::from_millis(1200));

    let base = "http://127.0.0.1:8123/";
    // C6: 从 demo/shell.json 读公钥（模拟客户端配置）
    let demo_cfg: shared::ShellConfig = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../demo/shell.json")),
        )
        .unwrap(),
    )
    .unwrap();
    let pubkey = demo_cfg.registry_pubkeys.values().next().unwrap().clone();
    println!("使用公钥(前8位): {}…", &pubkey[..8]);

    // 1. 拉清单(带签名校验)
    let mut pk = BTreeMap::new();
    pk.insert(base.to_string(), pubkey);
    let client = shared::RegistryClient::with_pubkeys(base, cache.clone(), pk);
    let (offline1, manifest) = client.load_manifest().unwrap();
    assert!(!offline1);
    assert!(!manifest.templates.is_empty());
    println!("1. manifest ok(签名通过), templates={}", manifest.templates.len());

    // 2. 签名校验拦截：用错公钥必须被拒
    let mut bad_pk = BTreeMap::new();
    bad_pk.insert(base.to_string(), "00".repeat(32));
    let bad_client = shared::RegistryClient::with_pubkeys(base, cache.clone(), bad_pk);
    assert!(bad_client.load_manifest().is_err(), "错误公钥应拒绝清单");
    println!("2. 错误公钥正确拦截签名校验");

    // 3. 惰性拉模板
    let (offline2, program) = client.load_template("dufs").expect("load template");
    assert!(!offline2);
    assert_eq!(program.id, "dufs");
    assert_eq!(program.binary, "dufs");
    // B4: 导入后快照来源戳
    assert!(program.template_source.as_deref().unwrap_or("").contains("dufs"), "应有 template_source");
    assert!(program.imported_at.is_some(), "应有 imported_at");
    println!("3. lazy template ok: {} (source={:?})", program.name, program.template_source);

    // 4. 停掉服务器 → 走缓存(离线回退)
    server.kill().ok();
    server.wait().ok();
    thread::sleep(Duration::from_millis(400));

    let (offline3, manifest2) = client.load_manifest().expect("offline manifest from cache");
    assert!(offline3, "网络断开后应标记离线并回退缓存");
    assert_eq!(manifest2.templates.len(), manifest.templates.len());
    // 只对已缓存过的模板可离线加载
    let (offline4, _prog) = client.load_template("dufs").expect("offline dufs from cache");
    assert!(offline4);
    // 未缓存过的模板离线应失败
    let uncached = client.load_template("syncthing");
    assert!(uncached.is_err(), "未缓存模板离线应失败");
    println!("4. offline fallback ok (缓存命中, 未缓存模板正确失败)");

    println!("ALL OK");
}