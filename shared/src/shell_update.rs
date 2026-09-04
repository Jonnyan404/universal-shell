//! 壳应用自身的更新检查：查询本仓库 GitHub Releases 最新 tag，
//! 与当前壳版本比较。有新版时返回下载页 URL，调用方（egui/Tauri）
//! 负责展示提示并引导用户前往下载（不做静默自动升级）。

use crate::github::GitHub;

/// 壳源码仓库（owner/repo），与侧栏 GitHub 链接保持一致。
pub const SHELL_REPO: &str = "Jonnyan404/universal-shell";

/// 发现的新版本信息。
#[derive(Debug, Clone)]
pub struct ShellUpdate {
    /// 当前壳版本（如 "0.1.0"）
    pub current: String,
    /// 远端最新 tag（如 "v0.1.1"）
    pub latest_tag: String,
    /// 对应 Release 页面 URL
    pub release_url: String,
}

/// 检查壳是否有新版：`Ok(None)` 表示已是最新（或查询失败时由调用方按失败处理）。
pub fn check_shell_update(
    current: &str,
    accelerate_prefix: &str,
    http_proxy: &str,
) -> anyhow::Result<Option<ShellUpdate>> {
    let mut gh = GitHub::default();
    gh.apply_network(accelerate_prefix, http_proxy);
    let latest = gh.latest(SHELL_REPO)?;
    if crate::version::is_newer(&latest.tag_name, current) {
        Ok(Some(ShellUpdate {
            current: current.to_string(),
            latest_tag: latest.tag_name.clone(),
            release_url: format!(
                "https://github.com/{SHELL_REPO}/releases/tag/{}",
                latest.tag_name
            ),
        }))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_url_points_at_tag_page() {
        let u = ShellUpdate {
            current: "0.1.0".into(),
            latest_tag: "v0.1.1".into(),
            release_url: format!("https://github.com/{SHELL_REPO}/releases/tag/v0.1.1"),
        };
        assert!(u.release_url.ends_with("/releases/tag/v0.1.1"));
        assert!(crate::version::is_newer(&u.latest_tag, &u.current));
    }
}
