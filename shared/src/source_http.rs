//! 通用 HTTP 直链下载源（方案 A）。
//!
//! 覆盖 GitHub release 之外的下载场景：自建文件站、厂商直链、Electron 系
//! latest.yml、自建 Gitea 等。与 [`crate::github::GitHub`] 平行，模板配
//! 置 `source.kind = "http"` 时使用：
//! - 版本：`version_url`（纯文本 / JSON 点路径 / 正则）
//! - 下载：`assets.<os>.urls` 直链模板（按序尝试）
//! - 校验：`sha256_url`（渲染 {version} 等占位符）；留空即显式免检

use anyhow::{anyhow, bail, Context};
use rust_i18n::t;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::{AssetRule, Program, SourceSpec};

pub struct HttpSource {
    pub client: reqwest::blocking::Client,
    /// 可选的 URL 加速代理前缀（如 https://gh-proxy.com/）
    pub accelerate_prefix: Option<String>,
}

impl Default for HttpSource {
    fn default() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new()),
            accelerate_prefix: None,
        }
    }
}

impl HttpSource {
    /// 应用网络设置：加速前缀会重写目标 URL；通用代理作用于所有请求。
    pub fn apply_network(&mut self, accelerate_prefix: &str, http_proxy: &str) {
        self.accelerate_prefix = if accelerate_prefix.trim().is_empty() {
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
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());
        }
    }

    fn accelerate(&self, url: &str) -> String {
        match &self.accelerate_prefix {
            Some(p) => format!("{}/{}", p.trim_end_matches('/'), url),
            None => url.to_string(),
        }
    }

    fn fetch_text(&self, url: &str) -> anyhow::Result<String> {
        let final_url = self.accelerate(url);
        let resp = self
            .client
            .get(&final_url)
            .header("User-Agent", "universal-shell")
            .send()
            .context(t!("err.github.request"))?;
        if !resp.status().is_success() {
            return Err(anyhow!(t!(
                "err.http.status",
                url = url,
                status = resp.status()
            )));
        }
        resp.text().context(t!("err.http.read"))
    }

    /// 从 version_url 提取版本串。
    /// 优先级：JSON 点路径 > 正则 > 整段文本(trim)。
    pub fn latest_version(&self, spec: &SourceSpec) -> anyhow::Result<String> {
        if spec.version_url.trim().is_empty() {
            bail!("{}", t!("err.http.no_version_url"));
        }
        let body = self.fetch_text(spec.version_url.trim())?;

        if !spec.version_json_path.trim().is_empty() {
            let v: serde_json::Value = serde_json::from_str(&body)
                .context(t!("err.http.parse_json", url = spec.version_url))?;
            let mut cur = &v;
            for seg in spec.version_json_path.split('.') {
                cur = cur
                    .get(seg.trim())
                    .ok_or_else(|| anyhow!(t!("err.http.json_missing", path = spec.version_json_path)))?;
            }
            let s = cur
                .as_str()
                .map(String::from)
                .or_else(|| cur.as_i64().map(|i| i.to_string()))
                .ok_or_else(|| anyhow!(t!("err.http.json_type", path = spec.version_json_path)))?;
            return Ok(s.trim().to_string());
        }

        if !spec.version_regex.trim().is_empty() {
            let re = regex::Regex::new(spec.version_regex.trim())
                .context(t!("err.http.bad_regex", re = spec.version_regex))?;
            let caps = re
                .captures(&body)
                .ok_or_else(|| anyhow!(t!("err.http.no_version", url = spec.version_url)))?;
            let m = caps
                .get(1)
                .or_else(|| caps.get(0))
                .ok_or_else(|| anyhow!(t!("err.http.no_version", url = spec.version_url)))?;
            return Ok(m.as_str().trim_start_matches('v').to_string());
        }

        let v = body.trim().trim_start_matches('v').to_string();
        if v.is_empty() {
            bail!("{}", t!("err.http.no_version", url = spec.version_url));
        }
        Ok(v)
    }

    /// 渲染 assets.<os>.urls 直链列表；返回 [(原始模板, 渲染后 URL)]。
    pub fn asset_urls(
        &self,
        program: &Program,
        version: &str,
        arch: &str,
    ) -> anyhow::Result<(AssetRule, Vec<String>)> {
        let rule = program
            .asset_rule_for_os()
            .cloned()
            .ok_or_else(|| anyhow!(t!("err.github.no_rule", os = std::env::consts::OS)))?;
        if rule.urls.is_empty() {
            bail!("{}", t!("err.http.no_urls"));
        }
        let rendered = rule
            .urls
            .iter()
            .map(|u| program.render_template(u, version, &rule, arch))
            .collect();
        Ok((rule, rendered))
    }

    /// 下载 sha256_url(渲染 {version}) 并解析出 hex 摘要；未配置 sha256_url 返回 None
    /// （显式免检；调用方若配置 check_sha256 钉住则仍校验）。
    pub fn declared_sha256(
        &self,
        program: &Program,
        spec: &SourceSpec,
        version: &str,
        arch: &str,
        rule: &AssetRule,
    ) -> anyhow::Result<Option<String>> {
        if spec.sha256_url.trim().is_empty() {
            return Ok(None);
        }
        let url = program.render_template(spec.sha256_url.trim(), version, rule, arch);
        let text = self.fetch_text(&url)?;
        // 文件可能含多个 whitespace 分隔 token（如 "<hex>  app-1.2.3.tar.gz"），取首段
        let hex = text.split_whitespace().next().unwrap_or("").trim().to_string();
        if hex.is_empty() {
            bail!("{}", t!("err.http.bad_sha", url = url));
        }
        Ok(Some(hex))
    }

    /// 从下载直链取文件名（URL 尾段；去查询串），用于临时文件命名。
    pub fn filename_from(url: &str) -> String {
        url.split('?')
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("asset.bin")
            .to_string()
    }

    /// 下载到目标路径（通用代理作用于请求本身）。
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
}