//! 注册表签名工具：对 `manifests.json` 生成 `<base>manifests.sig`（Ed25519。
//!
//! 用法：cargo run -p shared --example sign_registry -- <manifests.json> <seed_hex(64位)> [输出 sig 路径]
//!
//! 生成公钥(配置进 shell.json 的 registry_pubkeys)由 seed 导出：
//!   公钥 hex 会打印到 stdout，形如 `pubkey: <hex>`。
//!
//! 演示默认 seed：若未传则用固定演示 seed 并提示（生产请用随机生成）。
//!
//! 签名对象 = manifests.json 的原始文件字节（与 GitHub raw 拉取的内容一致）。

use std::path::PathBuf;

use shared::registry_sign::{sign_manifest, signing_key_from_seed_hex, pubkey_to_hex};

const DEMO_SEED: &str = "0000000000000000000000000000000000000000000000000000000000000001";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.len() > 3 {
        eprintln!("用法: sign_registry <manifests.json> [seed_hex] [out.sig]");
        std::process::exit(2);
    }
    let manifest_path = PathBuf::from(&args[0]);
    let seed = args.get(1).map(|s| s.as_str()).unwrap_or(DEMO_SEED);
    let out_path = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_path.with_extension("sig"));

    let manifest_bytes = std::fs::read(&manifest_path)
        .map_err(|e| anyhow::anyhow!("读取 {} 失败: {e}", manifest_path.display()))?;

    let key = signing_key_from_seed_hex(seed)?;
    let sig = sign_manifest(&manifest_bytes, &key);
    let pubkey = pubkey_to_hex(&ed25519_dalek::VerifyingKey::from(&key));

    std::fs::write(&out_path, format!("{sig}\n"))
        .map_err(|e| anyhow::anyhow!("写入 {} 失败: {e}", out_path.display()))?;
    println!("signed {} -> {}", manifest_path.display(), out_path.display());
    println!("pubkey: {pubkey}");
    println!(
        "hint: 在 shell.json 的 registry_pubkeys 中把该 base 映射到上面 pubkey 即启用签名校验"
    );
    Ok(())
}