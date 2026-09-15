//! GitHub API 交互：查询最新版本、下载资产。

use anyhow::{anyhow, Context};
use rust_i18n::t;
use std::path::PathBuf;
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::config::Program;

/// latest() 缓存有效期：60 秒——足够刷新界面又不至过度重复请求
const CACHE_TTL: Duration = Duration::from_secs(60);

/// 所有 GitHub 实例共享一个全局最新版本缓存，避免
/// get_status / batch_status / 15 秒轮询等反复发起网络请求。
static GLOBAL_CACHE: OnceLock<Mutex<BTreeMap<String, (Instant, LatestRelease)>>> = OnceLock::new();
/// 最近发行版列表缓存（键含 repo 与每页条数）
static LIST_CACHE: OnceLock<Mutex<BTreeMap<String, (Instant, Vec<LatestRelease>)>>> = OnceLock::new();
/// 按 tag 取单个发行版缓存
static TAG_CACHE: OnceLock<Mutex<BTreeMap<String, (Instant, LatestRelease)>>> = OnceLock::new();

fn global_cache() -> &'static Mutex<BTreeMap<String, (Instant, LatestRelease)>> {
    GLOBAL_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn list_cache() -> &'static Mutex<BTreeMap<String, (Instant, Vec<LatestRelease>)>> {
    LIST_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn tag_cache() -> &'static Mutex<BTreeMap<String, (Instant, LatestRelease)>> {
    TAG_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// 清空最新版本缓存（代理变更后调用，使新网络设置即时生效）
pub fn clear_github_cache() {
    if let Ok(mut c) = global_cache().lock() {
        c.clear();
    }
    if let Ok(mut c) = list_cache().lock() {
        c.clear();
    }
    if let Ok(mut c) = tag_cache().lock() {
        c.clear();
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct LatestRelease {
    pub tag_name: String,
    #[serde(default)]
    pub published_at: String,
    /// 是否为预发布(pre-release)。「pre」通道据此确认可跟随预发布版本
    #[serde(default)]
    pub prerelease: bool,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    #[serde(default)]
    pub browser_download_url: String,
    /// GitHub API 下发该资产的 sha256("sha256:<hex>")，用于下载后字节级校验
    #[serde(default)]
    pub digest: Option<String>,
}

pub struct GitHub {
    pub client: reqwest::blocking::Client,
    /// 可选的 GitHub API token，用于提升请求限额（5000/h vs 60/h）
    pub token: Option<String>,
    /// 可选的 URL 加速代理前缀，如 https://gh-proxy.com/
    pub proxy_prefix: Option<String>,
}

impl Default for GitHub {
    fn default() -> Self {
        let token = std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty());
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new()),
            token,
            proxy_prefix: None,
        }
    }
}

impl GitHub {
    /// 应用网络设置：加速前缀会重写目标 URL；通用代理作用于所有请求。
    pub fn apply_network(&mut self, accelerate_prefix: &str, http_proxy: &str) {
        self.proxy_prefix = if accelerate_prefix.trim().is_empty() {
            None
        } else {
            Some(accelerate_prefix.trim_end_matches('/').to_string())
        };
        if !http_proxy.trim().is_empty() {
            if let Ok(proxy) = reqwest::Proxy::all(http_proxy.trim()) {
                self.client = reqwest::blocking::Client::builder()
                    .proxy(proxy)
                    .timeout(Duration::from_secs(60))
                    .build()
                    .unwrap_or_else(|_| reqwest::blocking::Client::new());
            }
        } else {
            self.client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());
        }
    }

    /// 计算加速后的目标 URL（仅当配置了加速前缀时）
    fn accelerate(&self, url: &str) -> String {
        match &self.proxy_prefix {
            Some(p) => format!("{}/{}", p.trim_end_matches('/'), url),
            None => url.to_string(),
        }
    }

    /// 查询最新版本，返回 (tag, published_at)。结果带 TTL 缓存。
    pub fn latest(&self, repo: &str) -> anyhow::Result<LatestRelease> {
        // 1. 命中全局缓存直接返回
        if let Ok(c) = global_cache().lock() {
            if let Some((ts, rel)) = c.get(repo) {
                if ts.elapsed() < CACHE_TTL {
                    return Ok(rel.clone());
                }
            }
        }
        // 2. 网络请求（已设超时，不会无限挂起）
        let api = self.accelerate(&format!("https://api.github.com/repos/{repo}/releases/latest"));
        let mut req = self.client.get(&api).header("User-Agent", "universal-shell");
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let resp = req.send().context(t!("err.github.request"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(anyhow!(t!("err.github.api_status", status = status, body = body)));
        }
        let rel = resp
            .json::<LatestRelease>()
            .context(t!("err.github.parse"))?;
        // 3. 写入全局缓存
        if let Ok(mut c) = global_cache().lock() {
            c.insert(repo.to_string(), (Instant::now(), rel.clone()));
        }
        Ok(rel)
    }

    /// 列出仓库最近 N 个发行版（含预发布；按发布先后倒序）。结果带 TTL 缓存。
    /// 供「指定版本号更新/下载」的版本选择列表，为避免无限拉取，per_page 应传较小的值。
    pub fn releases(&self, repo: &str, per_page: usize) -> anyhow::Result<Vec<LatestRelease>> {
        let per = per_page.clamp(1, 100);
        let key = format!("{repo}\u{0}{per}");
        if let Ok(c) = list_cache().lock() {
            if let Some((ts, rel)) = c.get(&key) {
                if ts.elapsed() < CACHE_TTL {
                    return Ok(rel.clone());
                }
            }
        }
        let api = self.accelerate(&format!(
            "https://api.github.com/repos/{repo}/releases?per_page={per}"
        ));
        let mut req = self.client.get(&api).header("User-Agent", "universal-shell");
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let resp = req.send().context(t!("err.github.request"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(anyhow!(t!("err.github.api_status", status = status, body = body)));
        }
        let list = resp
            .json::<Vec<LatestRelease>>()
            .context(t!("err.github.parse"))?;
        if let Ok(mut c) = list_cache().lock() {
            c.insert(key, (Instant::now(), list.clone()));
        }
        Ok(list)
    }

    /// 按 tag 取单个发行版（含其资产），用于「钉住指定版本」的更新/下载。
    pub fn release_by_tag(&self, repo: &str, tag: &str) -> anyhow::Result<LatestRelease> {
        let key = format!("{repo}\u{0}{tag}");
        if let Ok(c) = tag_cache().lock() {
            if let Some((ts, rel)) = c.get(&key) {
                if ts.elapsed() < CACHE_TTL {
                    return Ok(rel.clone());
                }
            }
        }
        let api = self.accelerate(&format!(
            "https://api.github.com/repos/{repo}/releases/tags/{tag}"
        ));
        let mut req = self.client.get(&api).header("User-Agent", "universal-shell");
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let resp = req.send().context(t!("err.github.request"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(anyhow!(t!("err.github.api_status", status = status, body = body)));
        }
        let rel = resp
            .json::<LatestRelease>()
            .context(t!("err.github.parse"))?;
        if let Ok(mut c) = tag_cache().lock() {
            c.insert(key, (Instant::now(), rel.clone()));
        }
        Ok(rel)
    }

    /// 按更新通道解析「最新」发行版：
    /// - "pre"：取最近一条发行版（可能是预发布）
    /// - 其它（stable）：仅稳定版（`/releases/latest`）
    pub fn latest_channel(&self, repo: &str, channel: &str) -> anyhow::Result<LatestRelease> {
        if channel == "pre" {
            let mut list = self.releases(repo, 1)?;
            list.pop().ok_or_else(|| anyhow!(t!("err.github.no_release", repo = repo)))
        } else {
            self.latest(repo)
        }
    }

    /// 带进度回调的下载：逐块读取并回调 (bytes_received, total_bytes)。
    /// `total` 未知时为 0（如上游未给 Content-Length）。
    pub fn download_to_with_progress(
        &self,
        url: &str,
        dest: &PathBuf,
        on_progress: &dyn Fn(u64, u64),
    ) -> anyhow::Result<()> {
        use std::io::{Read, Write};
        let mut resp = self
            .client
            .get(url)
            .header("User-Agent", "universal-shell")
            .send()
            .context(t!("err.github.download"))?;
        if !resp.status().is_success() {
            return Err(anyhow!(t!("err.github.download_status", status = resp.status())));
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let total = resp.content_length().unwrap_or(0);
        let mut file = std::fs::File::create(dest)
            .with_context(|| t!("err.github.create_dest", path = dest.display()).to_string())?;
        let mut buf = [0u8; 64 * 1024];
        let mut received: u64 = 0;
        loop {
            let n = resp.read(&mut buf).context(t!("err.github.download"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).context(t!("err.github.download"))?;
            received += n as u64;
            on_progress(received, total);
        }
        Ok(())
    }

    /// 按候选列表对真实发行版资产做顺序匹配：
    /// 遍历 candidates，命中第一个真实存在者即返回 (资产名, 其下载 URL, 其 sha256 digest)。
    /// 全部未命中返回 Err(含提示)。
    pub fn match_candidate(
        &self,
        release: &LatestRelease,
        candidates: &[String],
    ) -> anyhow::Result<(String, String, Option<String>)> {
        for name in candidates {
            if let Some(asset) = release.assets.iter().find(|a| a.name == *name) {
                let raw = asset.browser_download_url.clone();
                let url = if let Some(p) = &self.proxy_prefix {
                    if !p.is_empty() {
                        format!("{}{}", p.trim_end_matches('/'), raw)
                    } else {
                        raw
                    }
                } else {
                    raw
                };
                return Ok((name.clone(), url, asset.digest.clone()));
            }
        }
        let have: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
        anyhow::bail!(
            "{}",
            t!(
                "err.github.no_match",
                candidates = format!("{:?}", candidates),
                count = have.len(),
                list = format!("{:?}", have.iter().take(5).copied().collect::<Vec<_>>())
            )
        )
    }

    /// 计算 Program 当前 OS/arch 需要的资产：对指定发行版做候选匹配，
    /// 返回 (rule, 资产名, URL, digest)。传入的 release 决定资产集合
    /// （钉住版本 / channel 最新均可），version 只用于渲染候选模板。
    pub fn resolve_download(
        &self,
        program: &Program,
        arch: &str,
        version: &str,
        release: &LatestRelease,
    ) -> anyhow::Result<(crate::config::AssetRule, String, String, Option<String>)> {
        let (rule, candidates) = program
            .candidate_names(arch, version)
            .ok_or_else(|| anyhow!(t!("err.github.no_rule", os = std::env::consts::OS)))?;
        let (name, url, digest) = self.match_candidate(release, &candidates)?;
        Ok((rule, name, url, digest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 解析 /releases、/releases/latest、/releases/tags/{tag} 的发行版对象（含预发布标记）。
    #[test]
    fn latest_release_parses_prerelease_flag() {
        let rel: LatestRelease = serde_json::from_str(
            r#"{"tag_name":"v2.0.0-rc.1","published_at":"2026-09-01T00:00:00Z","prerelease":true,
                "assets":[{"name":"x-darwin.tgz","browser_download_url":"https://.../x-darwin.tgz","digest":"sha256:abc"}]}"#,
        )
        .unwrap();
        assert_eq!(rel.tag_name, "v2.0.0-rc.1");
        assert!(rel.prerelease);
        assert_eq!(rel.assets.len(), 1);
        assert_eq!(rel.assets[0].digest.as_deref(), Some("sha256:abc"));

        // 旧响应无 prerelease 字段：默认 false
        let stable: LatestRelease = serde_json::from_str(
            r#"{"tag_name":"v1.0.0","published_at":"","assets":[]}"#,
        )
        .unwrap();
        assert!(!stable.prerelease);
    }
}