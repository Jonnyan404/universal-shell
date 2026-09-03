//! 下载资产校验：sha256 计算与比对。

use std::io::Read;
use std::path::Path;

use anyhow::{anyhow, bail, Context};
use rust_i18n::t;
use sha2::{Digest, Sha256};

/// 计算文件 sha256，返回小写 hex
pub fn sha256_hex(path: &Path) -> anyhow::Result<String> {
    let mut f = std::fs::File::open(path)
        .with_context(|| t!("err.checksum.open", path = path.display()).to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .with_context(|| t!("err.checksum.read", path = path.display()).to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 规范化期望摘要：接受 "sha256:abcd.." 或裸 hex
fn normalize_expected(s: &str) -> anyhow::Result<String> {
    let s = s.trim();
    let hex = match s.strip_prefix("sha256:") {
        Some(h) => h,
        None => s.strip_prefix("SHA256:").unwrap_or(s),
    };
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!(t!("err.checksum.invalid_sha", s = s));
    }
    Ok(hex.to_ascii_lowercase())
}

/// 校验下载文件。`expected` 为 GitHub digest 或模板 check_sha256 的原始值(可能带前缀)。
/// 不一致时返回带期望/实际值的错误；调用方应删除损坏文件。
pub fn verify_download(path: &Path, expected: &str) -> anyhow::Result<()> {
    let want = normalize_expected(expected)?;
    let got = sha256_hex(path)?;
    if got != want {
        return Err(anyhow!(t!(
            "err.checksum.mismatch",
            path = path.display(),
            want = want,
            got = got
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accepts_prefixed_and_bare() {
        let bare = "a".repeat(64);
        assert_eq!(normalize_expected(&format!("sha256:{bare}")).unwrap(), bare);
        assert_eq!(normalize_expected(&bare).unwrap(), bare);
        assert!(normalize_expected("abc").is_err());
    }

    #[test]
    fn sha256_of_known_bytes() {
        let p = std::env::temp_dir().join("cc-shared-sha-test");
        std::fs::write(&p, b"hello").unwrap();
        assert_eq!(
            sha256_hex(&p).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn verify_rejects_mismatch() {
        let p = std::env::temp_dir().join("cc-shared-sha-mismatch");
        std::fs::write(&p, b"hello").unwrap();
        // 把正确摘要首字符改掉，必须被拒
        let bad = format!("3cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        let err = verify_download(&p, &bad).unwrap_err();
        assert!(err.to_string().contains("sha256"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn verify_accepts_matching() {
        let p = std::env::temp_dir().join("cc-shared-sha-match");
        std::fs::write(&p, b"hello").unwrap();
        verify_download(
            &p,
            &format!("sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"),
        )
        .unwrap();
        let _ = std::fs::remove_file(&p);
    }
}