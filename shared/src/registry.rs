//! 远程模板注册表：两级(清单 + 模板惰性拉取) + 本地磁盘缓存(ETag/If-None-Match)。
//!
//! 约定目录结构(基地址以 `/` 结尾)：
//! - `<base>manifests.json` ：分类 + 模板索引(仅 id/name/category/描述/repo)
//! - `<base>templates/<id>.json` ：单模板完整定义，导入时才拉取(惰性)

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use rust_i18n::t;
use serde::{Deserialize, Serialize};

/// 清单里的单条模板索引(轻量，驱动浏览/搜索)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateIndex {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub repo: String,
}

/// 清单文件
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manifest {
    pub revision: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub templates: Vec<TemplateIndex>,
}

/// 注册表状态(供 UI 展示来源与时效)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistryState {
    pub url: String,
    pub manifest_revision: String,
    pub fetched_at: u64,
    pub offline: bool,
    pub template_count: usize,
}

pub struct RegistryClient {
    base: String,
    /// 缓存目录(下载成功即写盘)
    cache_dir: PathBuf,
    http: reqwest::blocking::Client,
    /// 可选：该注册表的 Ed25519 公钥(hex)。配置时加载清单强制验签。
    pubkeys: BTreeMap<String, String>,
}

impl RegistryClient {
    pub fn new(base: &str, cache_dir: PathBuf) -> Self {
        Self::with_network(base, cache_dir, BTreeMap::new(), None, None)
    }

    /// 带公钥映射创建。key 为 base URL（与配置里一致即可）。
    pub fn with_pubkeys(
        base: &str,
        cache_dir: PathBuf,
        pubkeys: BTreeMap<String, String>,
    ) -> Self {
        Self::with_network(base, cache_dir, pubkeys, None, None)
    }

    /// 完整构造：公钥 + 可选加速前缀（重写 base 用于下载）+ 可选通用代理。
    /// `accelerate_prefix` 会用于改写 `base`（仅当 base 无法直接访问时）；
    /// `http_proxy` 作用于该注册表的所有请求。
    pub fn with_network(
        base: &str,
        cache_dir: PathBuf,
        pubkeys: BTreeMap<String, String>,
        accelerate_prefix: Option<&str>,
        http_proxy: Option<&str>,
    ) -> Self {
        let mut client_builder = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(30));
        if let Some(proxy) = http_proxy {
            if !proxy.trim().is_empty() {
                if let Ok(p) = reqwest::Proxy::all(proxy.trim()) {
                    client_builder = client_builder.proxy(p);
                }
            }
        }
        let http = client_builder
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        // 加速前缀：重写 base，使清单/模板请求走加速地址
        let raw_base = base.trim_end_matches('/');
        let effective_base = match accelerate_prefix {
            Some(p) if !p.trim().is_empty() => {
                let prefix = p.trim_end_matches('/');
                let rewritten = if raw_base.starts_with("http") {
                    // 替换 origin 为加速前缀（保留路径）
                    let path = raw_base
                        .splitn(4, '/')
                        .nth(3)
                        .map(|s| format!("/{s}"))
                        .unwrap_or_else(|| "/".to_string());
                    format!("{prefix}{}", path.trim_end_matches('/').to_string() + "/")
                } else {
                    format!("{prefix}/{}", raw_base)
                };
                rewritten
            }
            _ => raw_base.to_string() + "/",
        };
        Self {
            base: effective_base,
            cache_dir,
            http,
            pubkeys,
        }
    }

    /// 该客户端是否有对该 base 配置的公钥
    fn configured_pubkey(&self) -> Option<&str> {
        for (k, v) in &self.pubkeys {
            if self.base.starts_with(k.trim_end_matches('/')) || self.base.as_str() == k.as_str() {
                return Some(v);
            }
        }
        None
    }

    fn cache_key(url: &str) -> String {
        // 文件名做缓存键：manifest 或 id
        if url.ends_with("manifests.json") {
            "manifest".into()
        } else if url.ends_with("manifests.sig") {
            "manifest.sig".into()
        } else if let Some(id) = url
            .rsplit('/')
            .next()
            .and_then(|f| f.strip_suffix(".json"))
        {
            id.to_string()
        } else {
            "unknown".into()
        }
    }

    fn cache_path(&self, key: &str) -> PathBuf {
        self.cache_dir.join(format!("{key}.json"))
    }

    fn etag_path(&self, key: &str) -> PathBuf {
        self.cache_dir.join(format!("{key}.etag"))
    }

    /// fetch：优先条件请求(If-None-Match)，304 用缓存；网络失败回退缓存(标记离线)。
    /// 返回 (是否命中缓存, 内容)。请求失败且有缓存时返回缓存内容。
    fn fetch_with_cache(&self, url: &str) -> (bool, String, u64) {
        let key = Self::cache_key(url);
        let cpath = self.cache_path(&key);
        let cached_body = std::fs::read_to_string(&cpath).ok();

        // 有缓存则带 ETag 做条件请求
        let resp = {
            let mut req = self
                .http
                .get(url)
                .header("User-Agent", "universal-shell")
                .header("Accept", "application/json");
            if let Ok(etag) = std::fs::read_to_string(self.etag_path(&key)) {
                let etag = etag.trim().to_string();
                if !etag.is_empty() {
                    req = req.header("If-None-Match", etag);
                }
            }
            req.send()
        };

        match resp {
            Ok(r) => {
                let status = r.status();
                if status.as_u16() == 304 {
                    let now = now_unix();
                    return (true, cached_body.unwrap_or_default(), now);
                }
                if !status.is_success() {
                    if let Some(body) = cached_body {
                        return (true, body, now_unix());
                    }
                    return (false, String::new(), now_unix());
                }
                // 成功：先取 ETag，再读 body
                let etag = r
                    .headers()
                    .get(reqwest::header::ETAG)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let body = r.text().unwrap_or_default();
                if body.is_empty() {
                    if let Some(b) = cached_body {
                        return (true, b, now_unix());
                    }
                } else {
                    let _ = std::fs::create_dir_all(&self.cache_dir);
                    let _ = std::fs::write(&cpath, &body);
                    let _ = std::fs::write(self.etag_path(&key), etag);
                }
                let now = now_unix();
                (false, body, now)
            }
            Err(_e) => {
                // 网络失败 → 回退缓存(可能为空)
                let body = cached_body.unwrap_or_default();
                let now = now_unix();
                (true, body, now)
            }
        }
    }

    /// 拉取清单。若对该 base 配置了公钥，则强制校验 `<base>manifests.sig`，
    /// 校验失败直接否决该清单。返回 (离线标记, 拉取/校验时刻(缓存日期), 清单)
    pub fn load_manifest(&self) -> anyhow::Result<(bool, u64, Manifest)> {
        let url = format!("{base}manifests.json", base = self.base);
        let (offline, text, fetched_at) = self.fetch_with_cache(&url);
        if let Some(pubkey) = self.configured_pubkey() {
            let sig_url = format!("{}manifests.sig", self.base);
            let (_off, sig_text, _) = self.fetch_with_cache(&sig_url);
            crate::registry_sign::verify_manifest(text.as_bytes(), sig_text.trim(), pubkey)
                .with_context(|| t!("err.registry.sig_rejected", base = &self.base).to_string())?;
        }
        let manifest: Manifest = serde_json::from_str(&text).context(t!("err.registry.parse_manifest"))?;
        if manifest.templates.is_empty() && offline {
            anyhow::bail!(t!("err.registry.empty_offline"));
        }
        Ok((offline, fetched_at, manifest))
    }

    /// 惰性拉取单个模板。返回 (离线标记, 模板 Program)
    pub fn load_template(&self, id: &str) -> anyhow::Result<(bool, crate::config::Program)> {
        let url = format!("{}templates/{id}.json", self.base);
        let (offline, text, _) = self.fetch_with_cache(&url);
        let mut program: crate::config::Program = serde_json::from_str(&text)
            .with_context(|| t!("err.registry.parse_template", id = id).to_string())?;
        // 打上来源戳，供「导入后快照」记录 template_source/imported_at
        program.template_source = Some(format!("{}{id}", self.base));
        program.imported_at = Some(now_unix());
        Ok((offline, program))
    }

    pub fn state(&self, manifest: &Manifest, offline: bool) -> RegistryState {
        RegistryState {
            url: self.base.clone(),
            manifest_revision: manifest.revision.clone(),
            fetched_at: now_unix(),
            offline,
            template_count: manifest.templates.len(),
        }
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 把多个 registry 的清单按模板 id 去重(后者覆盖)，便于全局搜索。
pub fn merge_indexes(
    sources: Vec<(String, Manifest)>,
) -> BTreeMap<String, (String, TemplateIndex)> {
    let mut map = BTreeMap::new();
    for (base, manifest) in sources {
        for t in manifest.templates {
            map.insert(t.id.clone(), (base.clone(), t));
        }
    }
    map
}

/// 多注册表合并结果：模板 id -> (首选来源 base, 索引)，冲突与各源状态一并记录。
#[derive(Debug, Clone, Default)]
pub struct MergedSource {
    /// id -> (base, 索引)。同 id 多源时按配置文件登记顺序取第一个(前置优先)。
    pub by_id: BTreeMap<String, (String, TemplateIndex)>,
    /// 各源 (base, 离线标记, 缓存日期/拉取时刻)
    pub sources: Vec<(String, bool, u64)>,
    /// 冲突：id -> 提供它的 base 列表(顺序保持配置顺序)
    pub conflicts: Vec<(String, Vec<String>)>,
}

impl MergedSource {
    pub fn template_count(&self) -> usize {
        self.by_id.len()
    }

    /// 某 id 是否来自多个注册表
    pub fn id_conflicts(&self, id: &str) -> Vec<&str> {
        self.conflicts
            .iter()
            .find(|(tid, _)| tid == id)
            .map(|(_, bases)| bases.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }
}

/// 拉取多个清单并合并。任一源失败不阻断其它：该源标记离线(缓存优先)。
/// `pubkeys` 为 base(或前缀) -> Ed25519 公钥 hex，命中即强制验签。
/// `accelerate_prefix` / `http_proxy` 为可选网络设置（见 [`RegistryClient::with_network`]）。
pub fn load_merged_manifests(
    bases: &[String],
    cache_dir: PathBuf,
    pubkeys: BTreeMap<String, String>,
    accelerate_prefix: Option<&str>,
    http_proxy: Option<&str>,
) -> MergedSource {
    let mut merged = MergedSource::default();
    for base in bases {
        let client = RegistryClient::with_network(
            base,
            cache_dir.clone(),
            pubkeys.clone(),
            accelerate_prefix,
            http_proxy,
        );
        let (offline, fetched_at, manifest) = client.load_manifest().unwrap_or((true, now_unix(), Manifest::default()));
        merged.sources.push((base.clone(), offline, fetched_at));
        merge_manifest_into(&mut merged, base, &manifest);
    }
    merged
}

/// 把单个清单合并进 MergedSource（追踪冲突；同 id 取先登记源）。
fn merge_manifest_into(merged: &mut MergedSource, base: &str, manifest: &Manifest) {
    for t in &manifest.templates {
        match merged.by_id.get(&t.id) {
            None => {
                merged.by_id.insert(t.id.clone(), (base.to_string(), t.clone()));
            }
            Some((first_base, _)) => {
                // 冲突：登记 (id, [首个来源, 新来源…])
                if let Some((_, bases)) = merged.conflicts.iter_mut().find(|(id, _)| *id == t.id) {
                    if !bases.contains(&base.to_string()) {
                        bases.push(base.to_string());
                    }
                } else {
                    merged.conflicts
                        .push((t.id.clone(), vec![first_base.clone(), base.to_string()]));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(id: &str) -> TemplateIndex {
        TemplateIndex {
            id: id.into(),
            name: id.into(),
            category: "test".into(),
            description: String::new(),
            repo: "a/b".into(),
        }
    }

    fn manifest(ids: &[&str]) -> Manifest {
        Manifest {
            revision: "r".into(),
            categories: vec!["test".into()],
            templates: ids.iter().map(|s| idx(s)).collect(),
        }
    }

    #[test]
    fn merge_conflict_tracks_first_and_second_source() {
        let mut m = MergedSource::default();
        let m1 = manifest(&["dufs", "frpc"]);
        let m2 = manifest(&["dufs", "croc"]);
        merge_manifest_into(&mut m, "https://a/registry/", &m1);
        merge_manifest_into(&mut m, "https://b/registry/", &m2);
        assert_eq!(m.by_id.len(), 3);
        // 冲突只针对 dufs
        assert_eq!(m.conflicts.len(), 1);
        let (cid, bases) = &m.conflicts[0];
        assert_eq!(cid, "dufs");
        assert_eq!(bases, &vec!["https://a/registry/".to_string(), "https://b/registry/".to_string()]);
        // 首源保留优先级
        assert_eq!(m.by_id.get("dufs").unwrap().0, "https://a/registry/");
        assert_eq!(m.id_conflicts("dufs"), vec!["https://a/registry/", "https://b/registry/"]);
        // 非冲突 id
        assert!(m.id_conflicts("croc").is_empty());
    }
}