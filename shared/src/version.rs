//! 版本号比较工具（GitHub release tag 等）。

/// 返回版本号 `a` 是否严格新于 `b`。
/// 兼容形如 `v1.2.3` / `1.0.0-beta1` / `2024.05` 的常见 tag。
pub fn is_newer(a: &str, b: &str) -> bool {
    let a = a.trim_start_matches('v');
    let b = b.trim_start_matches('v');
    let va: Vec<&str> = a.split('.').collect();
    let vb: Vec<&str> = b.split('.').collect();

    for i in 0..va.len().max(vb.len()) {
        let na = va.get(i).map(|s| parse_num(s)).unwrap_or(0);
        let nb = vb.get(i).map(|s| parse_num(s)).unwrap_or(0);
        if na != nb {
            return na > nb;
        }
    }

    // 数字段均相等：纯数字段为正版，带后缀（如 -beta）视为更旧
    let stable_a = is_stable(va.last().copied().unwrap_or(""));
    let stable_b = is_stable(vb.last().copied().unwrap_or(""));
    match (stable_a, stable_b) {
        (true, true) => false,
        (true, false) => true,
        (false, true) => false,
        (false, false) => a > b,
    }
}

fn parse_num(s: &str) -> u64 {
    s.chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn is_stable(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_basic() {
        assert!(is_newer("1.2.3", "1.2.2"));
        assert!(!is_newer("1.2.2", "1.2.3"));
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(is_newer("v1.9.0", "v1.10.0") == false);
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(is_newer("1.2.4", "1.2"));
        assert!(is_newer("1.2.0", "1.2.0-beta2"));
        assert!(!is_newer("1.2.0-beta2", "1.2.0"));
        assert!(is_newer("2024.05", "2024.04"));
    }
}