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

fn global_cache() -> &'static Mutex<BTreeMap<String, (Instant, LatestRelease)>> {
    GLOBAL_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// 清空最新版本缓存（代理变更后调用，使新网络设置即时生效）
pub fn clear_github_cache() {
    if let Ok(mut c) = global_cache().lock() {
        c.clear();
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct LatestRelease {
    pub tag_name: String,
    #[serde(default)]
    pub published_at: String,
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

    /// 找匹配 asset 的下载直链(可走加速前缀)
    pub fn find_asset_url(
        &self,
        release: &LatestRelease,
        want_filename: &str,
    ) -> anyhow::Result<String> {
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == want_filename)
            .ok_or_else(|| anyhow!(t!("err.github.no_asset", name = want_filename)))?;
        let raw = asset.browser_download_url.clone();
        if let Some(p) = &self.proxy_prefix {
            if !p.is_empty() {
                return Ok(format!("{}{}", p.trim_end_matches('/'), raw));
            }
        }
        Ok(raw)
    }

    /// 下载资产到目标路径（URL 已含加速前缀；仅通用代理作用于请求本身）
    pub fn download_to(&self, url: &str, dest: &PathBuf) -> anyhow::Result<()> {
        self.download_to_with_progress(url, dest, &|_, _| {})
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
    pub fn match_candidate<'a>(
        &self,
        release: &'a LatestRelease,
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

    /// 计算 Program 当前 OS/arch 需要的资产：候选匹配后返回 (rule, 资产名, URL, digest)
    pub fn resolve_download(
        &self,
        program: &Program,
        arch: &str,
        version: &str,
    ) -> anyhow::Result<(crate::config::AssetRule, String, String, Option<String>)> {
        let release = self.latest(&program.repo)?;
        let (rule, candidates) = program
            .candidate_names(arch, version)
            .ok_or_else(|| anyhow!(t!("err.github.no_rule", os = std::env::consts::OS)))?;
        let (name, url, digest) = self.match_candidate(&release, &candidates)?;
        Ok((rule, name, url, digest))
    }
}