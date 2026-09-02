//! 配置模型：描述一个「被管理的程序」(program)。
//!
//! 配置驱动 UI：`fields` 数组决定界面渲染哪些控件(文本框/文件路径/目录/复选框),
//! `args` 模板决定启动时如何把字段值拼进命令行参数。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 可复用的「需要下载的资产」描述。按 OS + 架构匹配。
#[derive(Debug, Clone, Serialize)]
pub struct AssetRule {
    /// 候选文件名模板（按序尝试，第一个真实存在者胜出）。
    /// 支持占位符：{name} {version} {arch} {os} {ext}
    pub candidates: Vec<String>,
    /// 压缩包封装：tar.gz / zip / raw(裸二进制)
    pub format: String,
    /// 解压模式：single(抽单成员) / whole(整包解到 id 目录) / raw
    pub mode: ExtractMode,
    /// 目标可执行文件：
    /// - single 模式：包内成员名（留空取第一个非目录成员）
    /// - whole 模式：解包目录内相对路径（如 "syncthing-macos-arm64-v2.1.3/syncthing"）
    pub member: Option<String>,
}

impl<'de> Deserialize<'de> for AssetRule {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            candidates: Vec<String>,
            #[serde(default)]
            filename: Option<String>,
            #[serde(default = "default_format")]
            format: String,
            #[serde(default)]
            mode: ExtractMode,
            #[serde(default)]
            member: Option<String>,
        }
        let r = Raw::deserialize(d)?;
        let candidates = if !r.candidates.is_empty() {
            r.candidates
        } else if let Some(f) = r.filename {
            vec![f]
        } else {
            Vec::new()
        };
        Ok(AssetRule {
            candidates,
            format: r.format,
            mode: r.mode,
            member: r.member,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtractMode {
    Single,
    Whole,
    Raw,
}

impl Default for ExtractMode {
    fn default() -> Self {
        ExtractMode::Single
    }
}

fn default_format() -> String {
    "tar.gz".to_string()
}

/// 字段定义。`kind` 决定 UI 控件类型。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldKind {
    /// 普通文本框
    String {
        label: String,
        #[serde(default)]
        default: String,
        #[serde(default)]
        placeholder: String,
    },
    /// 文件路径选择器
    File {
        label: String,
        #[serde(default)]
        default: String,
        /// rfd 文件过滤，如 "*.json"
        #[serde(default)]
        filter: String,
    },
    /// 目录路径选择器
    Directory {
        label: String,
        #[serde(default)]
        default: String,
    },
    /// 复选框
    Boolean {
        label: String,
        #[serde(default)]
        default: bool,
    },
    /// 开机启动复选框(特殊:写入系统 LoginItem / 自启配置)
    #[serde(rename = "autostart")]
    AutoStart {
        label: String,
        #[serde(default)]
        default: bool,
    },
}

/// 一个受管程序
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    /// 唯一 id，也用作配置文件名
    pub id: String,
    /// 显示名称
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 模板分类(文件共享/代理/存储…)，仅模板使用
    #[serde(default)]
    pub category: String,
    /// GitHub repo，如 "Jonnyan404/cloud-clipboard-go"
    pub repo: String,
    /// 下载后落盘的可执行文件名(刻意与壳不同名，避免覆盖壳自身)
    pub binary: String,
    #[serde(default = "default_os_assets")]
    pub assets: BTreeMap<String, AssetRule>,
    /// 架构名映射：本机 arch -> 上游资产里的 arch token
    #[serde(default)]
    pub arch_map: BTreeMap<String, String>,
    /// OS 名映射：本机 OS -> 上游资产里的 os token（如 macOS -> "darwin"/"osx"/"apple-darwin"）
    #[serde(default)]
    pub os_map: BTreeMap<String, String>,
    /// UI 字段
    pub fields: Vec<Field>,
    /// 启动参数模板，如 ["-port", "{port}", "-config", "{config}"]
    pub args: Vec<String>,
    #[serde(default = "default_working_dir")]
    pub working_dir: String,
    /// 来源模板 id(导入时记录，表示「此程序由模板 <default?> 产生」)
    #[serde(default)]
    pub template_source: Option<String>,
    /// 导入时间(unix 秒)
    #[serde(default)]
    pub imported_at: Option<u64>,
    /// 期望资产 sha256(可选)。模板可为固定版本钉住哈希；缺省时用 GitHub API 返回的
    /// asset digest 校验，防止篡改资产被安装。
    #[serde(default)]
    pub check_sha256: Option<String>,
    /// 是否在侧栏/主管理列表隐藏（批量管理仍可见）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// 从模板导入到 ShellConfig 时打上的来源戳
pub fn stamp_from_template(
    program: &mut Program,
    registry_url: &str,
    template_id: &str,
    now: u64,
) {
    program.template_source = Some(format!("{registry_url}#{template_id}"));
    program.imported_at = Some(now);
}

/// 一个 UI 字段 = 定义的 kind + 运行时值
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    pub key: String,
    #[serde(flatten)]
    pub kind: FieldKind,
}

impl Field {
    pub fn label(&self) -> &str {
        match &self.kind {
            FieldKind::String { label, .. }
            | FieldKind::File { label, .. }
            | FieldKind::Directory { label, .. }
            | FieldKind::Boolean { label, .. }
            | FieldKind::AutoStart { label, .. } => label,
        }
    }

    pub fn default_raw(&self) -> String {
        match &self.kind {
            FieldKind::String { default, .. } | FieldKind::File { default, .. } | FieldKind::Directory { default, .. } => default.clone(),
            FieldKind::Boolean { default, .. } | FieldKind::AutoStart { default, .. } => {
                if *default { "true".into() } else { "false".into() }
            }
        }
    }
}

fn default_os_assets() -> BTreeMap<String, AssetRule> {
    use ExtractMode::Single;
    let mut m = BTreeMap::new();
    m.insert(
        "darwin".into(),
        AssetRule {
            candidates: vec!["{name}_Darwin_{arch}.{ext}".into()],
            format: "tar.gz".into(),
            mode: Single,
            member: None,
        },
    );
    m.insert(
        "linux".into(),
        AssetRule {
            candidates: vec!["{name}_Linux_{arch}.{ext}".into()],
            format: "tar.gz".into(),
            mode: Single,
            member: None,
        },
    );
    m.insert(
        "windows".into(),
        AssetRule {
            candidates: vec!["{name}_Windows_{arch}.zip".into()],
            format: "zip".into(),
            mode: Single,
            member: None,
        },
    );
    m
}

fn default_working_dir() -> String {
    ".".into()
}

/// 用于模板渲染的 OS token。默认映射：macOS -> "darwin"
pub fn os_key() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    }
}

impl Program {
    /// 渲染 args 模板，field_values 提供 {key} 的展开
    pub fn render_args(&self, field_values: &BTreeMap<String, String>) -> Vec<String> {
        self.args
            .iter()
            .map(|t| {
                let mut s = t.clone();
                for (k, v) in field_values {
                    s = s.replace(&format!("{{{k}}}"), v);
                }
                s
            })
            .collect()
    }

    /// 当前 OS/arch 对应的资产规则(仅取规则，不做网络匹配)
    pub fn asset_rule_for_os(&self) -> Option<&AssetRule> {
        self.assets.get(os_key())
    }

    /// 渲染任意模板串，替换 {name} {version} {arch} {os} {ext}
    pub fn render_template(
        &self,
        template: &str,
        version: &str,
        rule: &AssetRule,
        arch_token: &str,
    ) -> String {
        let arch = self.arch_map.get(arch_token).cloned().unwrap_or_else(|| arch_token.to_string());
        let os = self.os_map.get(os_key()).cloned().unwrap_or_else(|| os_key().to_string());
        let nametok = self.id.clone();
        let ext = if rule.format == "tar.gz" { "tar.gz" } else { rule.format.as_str() };
        template
            .replace("{name}", &nametok)
            .replace("{version}", version)
            .replace("{arch}", &arch)
            .replace("{os}", &os)
            .replace("{ext}", ext)
    }

    /// 全部候选(按序)渲染后的文件名，供下载匹配。
    /// 优先把包含当前 arch token 的候选取前，避免多架构模板命中错误架构。
    pub fn candidate_names(&self, arch: &str, version: &str) -> Option<(AssetRule, Vec<String>)> {
        let rule = self.assets.get(os_key())?.clone();
        let arch_token = self.arch_map.get(arch).cloned().unwrap_or_else(|| arch.to_string());
        let mut names: Vec<(String, bool)> = rule
            .candidates
            .iter()
            .map(|t| self.render_template(t, version, &rule, arch))
            .map(|n| {
                let hits = n.contains(&arch_token);
                (n, hits)
            })
            .collect();
        // 稳定排序：含本机 arch 的排前
        names.sort_by_key(|(_, hits)| std::cmp::Reverse(*hits));
        Some((rule, names.into_iter().map(|(n, _)| n).collect()))
    }

    /// 各自治装的 UI 字段运行时值
    pub fn runtime_defaults(&self) -> BTreeMap<String, String> {
        self.fields
            .iter()
            .map(|f| (f.key.clone(), f.default_raw()))
            .collect()
    }
}

/// 壳的全局配置：受管程序列表 + 数据目录
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShellConfig {
    pub programs: Vec<Program>,
    /// 远程模板注册表基地址列表(如 "https://…/templates/"，以 / 结尾)
    #[serde(default)]
    pub template_registries: Vec<String>,
    /// 各注册表 base(或前缀) -> Ed25519 公钥(hex)。命中即对该源强制验签。
    #[serde(default)]
    pub registry_pubkeys: BTreeMap<String, String>,
}

impl ShellConfig {
    pub fn load(path: &PathBuf) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Self = serde_json::from_str(&raw)?;
        Ok(cfg)
    }
}