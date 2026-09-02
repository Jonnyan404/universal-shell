//! 注册表签名：Ed25519 验签与签名工具逻辑。
//!
//! 发布方对 `manifests.json` 的原始字节签出 `<base>manifests.sig`（hex），
//! 客户端在配置里带该注册表公钥时，拉取清单后先验签再使用。

use anyhow::{anyhow, bail, Context};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

pub fn pubkey_to_hex(pk: &VerifyingKey) -> String {
    hex(&pk.to_bytes())
}

pub fn verify_manifest(manifest_bytes: &[u8], sig_hex: &str, pubkey_hex: &str) -> anyhow::Result<()> {
    let pk = VerifyingKey::from_bytes(&to_array::<32>(&dehex(pubkey_hex)?)?)
        .context("公钥格式非法（应为 32 字节 hex）")?;
    let sig_bytes = to_array::<64>(&dehex(sig_hex)?)?;
    pk.verify_strict(manifest_bytes, &Signature::from_bytes(&sig_bytes))
        .map_err(|_| anyhow!("清单签名校验失败：内容或签名与公钥不匹配"))
}

fn to_array<const N: usize>(v: &[u8]) -> anyhow::Result<[u8; N]> {
    v.try_into()
        .map_err(|_| anyhow!("期望 {N} 字节，实际 {}", v.len()))
}

pub fn sign_manifest(manifest_bytes: &[u8], signing_key: &SigningKey) -> String {
    let sig = signing_key.sign(manifest_bytes);
    hex(&sig.to_bytes())
}

/// 从 seed（32 字节 hex）恢复签名密钥，便于仓库内可复现
pub fn signing_key_from_seed_hex(seed_hex: &str) -> anyhow::Result<SigningKey> {
    let seed: [u8; 32] = dehex(seed_hex)?
        .try_into()
        .map_err(|_| anyhow!("seed 应为 32 字节（64 hex）"))?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn dehex(s: &str) -> anyhow::Result<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let s = s.trim();
    if s.len() % 2 != 0 {
        bail!("hex 长度必须是偶数");
    }
    s.as_bytes()
        .chunks(2)
        .map(|w| Ok((val(w[0]).ok_or_else(|| anyhow!("非法 hex: {s}"))? << 4)
            | val(w[1]).ok_or_else(|| anyhow!("非法 hex: {s}"))?))
        .collect()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_sign_verify_and_tamper_reject() {
        // 固定 seed，避免测试依赖 RNG
        let seed: [u8; 32] = [7u8; 32];
        let key = SigningKey::from_bytes(&seed);
        let pk = VerifyingKey::from(&key);
        let msg = b"{\"revision\":\"v1\",\"templates\":[]}";
        let sig = sign_manifest(msg, &key);
        verify_manifest(msg, &sig, &pubkey_to_hex(&pk)).unwrap();
        // 篡改：消息变一个字节必须被拒
        let tampered = b"{\"revision\":\"v2\",\"templates\":[]}";
        assert!(verify_manifest(tampered, &sig, &pubkey_to_hex(&pk)).is_err());
    }

    #[test]
    fn dehex_roundtrip() {
        assert_eq!(dehex("deadBEEF").unwrap(), vec![0xde, 0xad, 0xbe, 0xef]);
        assert!(dehex("abc").is_err());
        assert!(dehex("zz").is_err());
    }
}