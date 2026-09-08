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
    /// HTTP 直链源(方案 A)的可选下载 URL 模板列表。按序尝试，第一个能下载者胜出。
    /// 支持占位符：{name} {version} {arch} {os} {ext}
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<String>,
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
            urls: Vec<String>,
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
            urls: r.urls,
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
    /// 远程源标识。可为 GitHub repo(如 "Jonnyan404/cloud-clipboard-go")，
    /// 或其它 HTTP 源 provider 的标识符；空串表示「本地程序」——壳不下载/不更新，
    /// 直接使用 `binary` 指定的可执行文件(可为其绝对路径)。
    #[serde(default)]
    pub repo: String,
    /// 下载后落盘的可执行文件名(刻意与壳不同名，避免覆盖壳自身)
    pub binary: String,
    /// 下载源。缺省(无 source / 非 http)走现有 GitHub release 逻辑。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceSpec>,
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
    /// 启动时注入的环境变量。`value` 支持 {field_key} 展开（与 args 同一套替换），
    /// 无占位符即为常量。env 值全程不进日志、不写 shell 配置，只在进程启动时注入。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<EnvVar>,
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

/// HTTP 直链下载源（方案 A：通用 HTTP 源）。
/// 模板配置 `source.kind = "http"` 时，下载/更新/版本检查走直链：
/// - 版本：`version_url`(纯文本 / JSON 点路径 / 正则)
/// - 下载：`assets.<os>.urls` 直链模板列表(按序尝试)
/// - 校验：`sha256_url`(渲染 {version} 等占位符)；留空即"显式免检"。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceSpec {
    /// 源类型标识，当前仅 "http"
    pub kind: String,
    /// 版本探测 URL：GET 后按 version_json_path / version_regex / 纯文本 提取版本
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version_url: String,
    /// 从 version_url 响应 JSON 里按点路径取值，如 "version" / "app.version"
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version_json_path: String,
    /// 或：正则提取(如 `v([\d.]+)`)，命中即取第一组/整体
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version_regex: String,
    /// 可选 sha256 校验 URL(支持 {version} 等占位符)。留空 = 显式免校验
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256_url: String,
}

impl SourceSpec {
    pub fn is_http(&self) -> bool {
        self.kind == "http"
    }
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
    /// 启动前必填标记：为 true 且值为空时，启动前报错并列出缺项
    #[serde(default, skip_serializing_if = "is_falsef")]
    pub required: bool,
}

/// 字段的运行时值（含默认）。
fn is_falsef(b: &bool) -> bool {
    !*b
}

/// 环境变量定义。`value` 为模板串：`{field_key}` 由字段运行时值展开，
/// 不含占位符即为常量（如 `RUST_LOG=info`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnvVar {
    /// 环境变量名（如 CROC_SECRET / HTTP_PROXY）
    pub key: String,
    /// 值模板，支持 {field_key} 展开
    pub value: String,
    /// UI 展示标签（可选，空时前端用 key 作标签）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
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

pub fn default_os_assets() -> BTreeMap<String, AssetRule> {
    use ExtractMode::Single;
    let mut m = BTreeMap::new();
    m.insert(
        "darwin".into(),
        AssetRule {
            candidates: vec!["{name}_Darwin_{arch}.{ext}".into()],
            urls: vec![],
            format: "tar.gz".into(),
            mode: Single,
            member: None,
        },
    );
    m.insert(
        "linux".into(),
        AssetRule {
            candidates: vec!["{name}_Linux_{arch}.{ext}".into()],
            urls: vec![],
            format: "tar.gz".into(),
            mode: Single,
            member: None,
        },
    );
    m.insert(
        "windows".into(),
        AssetRule {
            candidates: vec!["{name}_Windows_{arch}.zip".into()],
            urls: vec![],
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
    /// 序列化为「模板比对」用规范 JSON 文本：去掉纯运行时字段
    /// （template_source / imported_at / hidden），只留下模板结构本身。
    /// struct 字段与 BTreeMap 的序列化顺序确定，同一内容两次输出一致。
    pub fn diff_json(&self) -> String {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if let Some(obj) = v.as_object_mut() {
            obj.remove("template_source");
            obj.remove("imported_at");
            obj.remove("hidden");
        }
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }

    /// 渲染 args 模板，field_values 提供 {key} 的展开
    pub fn render_args(&self, field_values: &BTreeMap<String, String>) -> Vec<String> {
        let mut out = Vec::new();
        for t in &self.args {
            let mut s = t.clone();
            for (k, v) in field_values {
                s = s.replace(&format!("{{{k}}}"), v);
            }
            // `args_extra` 是自由命令尾：按 shell 规则分词（支持引号/转义），
            // 否则 "--code 123" 会作为一个 argv 参数传给程序，Go flag 等解析必炸。
            // 其它字段（如路径）保持单 token，含空格也不拆。空串/未闭合引号不产出参数。
            if t.contains("{args_extra}") {
                match shlex::split(&s) {
                    Some(toks) => out.extend(toks),
                    None if s.trim().is_empty() => {}
                    None => out.push(s),
                }
            } else {
                out.push(s);
            }
        }
        out
    }

    /// 渲染环境变量：`{field_key}` 展开为字段运行时值。
    /// 空 key / 展开后为空串的条目不产出（避免误设空环境变量）。
    pub fn render_env(&self, field_values: &BTreeMap<String, String>) -> Vec<(String, String)> {
        self.env
            .iter()
            .map(|e| {
                let mut v = e.value.clone();
                for (k, fv) in field_values {
                    v = v.replace(&format!("{{{k}}}"), fv);
                }
                (e.key.clone(), v)
            })
            .filter(|(k, v)| !k.is_empty() && !v.is_empty())
            .collect()
    }

    /// 当前 OS 对应的资产规则(仅取规则，不做网络匹配)
    pub fn asset_rule_for_os(&self) -> Option<&AssetRule> {
        self.assets.get(os_key())
    }

    /// 渲染任意模板串，替换 {name} {version} {arch} {os} {ext}。
    /// {name} 取二进制名 binary（复制模板 id 会变、binary 不变，下载链接才不会跟 id 跑）；
    /// binary 为空时回退 id。
    pub fn render_template(
        &self,
        template: &str,
        version: &str,
        rule: &AssetRule,
        arch_token: &str,
    ) -> String {
        let arch = self.arch_map.get(arch_token).cloned().unwrap_or_else(|| arch_token.to_string());
        let os = self.os_map.get(os_key()).cloned().unwrap_or_else(|| os_key().to_string());
        let nametok = if self.binary.is_empty() { self.id.clone() } else { self.binary.clone() };
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn prog(args: Vec<String>) -> Program {
        Program {
            id: "x".into(),
            name: "x".into(),
            description: String::new(),
            category: String::new(),
            repo: String::new(),
            source: None,
            binary: "x".into(),
            assets: default_os_assets(),
            arch_map: BTreeMap::new(),
            os_map: BTreeMap::new(),
            fields: vec![],
            args,
            env: vec![],
            working_dir: ".".into(),
            template_source: None,
            imported_at: None,
            check_sha256: None,
            hidden: false,
        }
    }

    /// args_extra 按 shell 分词；其它字段值(含空格)保持单参数。
    #[test]
    fn render_args_splits_args_extra_but_keeps_other_fields_whole() {
        let p = prog(vec!["--flag".into(), "{path}".into(), "{args_extra}".into()]);
        let mut fv = BTreeMap::new();
        fv.insert("path".into(), "/my dir/a".into());
        fv.insert("args_extra".into(), "--code abc \"/quoted dir/x\"".into());
        assert_eq!(
            p.render_args(&fv),
            vec!["--flag", "/my dir/a", "--code", "abc", "/quoted dir/x"]
        );
    }

    /// 空 args_extra 不产出空参数。
    #[test]
    fn render_args_drops_empty_args_extra() {
        let p = prog(vec!["send".into(), "{args_extra}".into()]);
        let mut fv = BTreeMap::new();
        fv.insert("args_extra".into(), "".into());
        assert_eq!(p.render_args(&fv), vec!["send"]);
    }

    /// env：{field} 展开，常量原样，空 key/空值剔除。
    #[test]
    fn render_env_substitutes_fields_and_drops_empties() {
        let p = Program {
            id: "x".into(),
            name: "x".into(),
            description: String::new(),
            category: String::new(),
            repo: String::new(),
            source: None,
            binary: "x".into(),
            assets: default_os_assets(),
            arch_map: BTreeMap::new(),
            os_map: BTreeMap::new(),
            fields: vec![],
            args: vec![],
            env: vec![
                EnvVar { key: "CROC_SECRET".into(), value: "croc-{code}".into(), label: String::new() },
                EnvVar { key: "RUST_LOG".into(), value: "info".into(), label: String::new() },
                EnvVar { key: "BLANK".into(), value: String::new(), label: String::new() },
                EnvVar { key: String::new(), value: "x".into(), label: String::new() },
            ],
            working_dir: ".".into(),
            template_source: None,
            imported_at: None,
            check_sha256: None,
            hidden: false,
        };
        let mut fv = BTreeMap::new();
        fv.insert("code".into(), "12345678".into());
        assert_eq!(
            p.render_env(&fv),
            vec![
                ("CROC_SECRET".to_string(), "croc-12345678".to_string()),
                ("RUST_LOG".to_string(), "info".to_string()),
            ]
        );
    }

    /// 内容特征 + stderr 标记都可判为错误行。
    #[test]
    fn is_err_line_detects_content_and_marker() {
        assert!(is_err_line("Incorrect Usage: flag provided but not defined: -code 123"));
        assert!(is_err_line("Error: boom"));
        assert!(is_err_line("USAGE:\n  croc send ..."));
        assert!(is_err_line(&('\u{1f}'.to_string() + "fatal: x")));
        // stderr 进度行（croc Sending… 走 stderr）只按内容判，不标红
        assert!(!is_err_line(&('\u{1f}'.to_string() + "Sending 'f.txt' (53 B)")));
        assert!(!is_err_line(&('\u{1f}'.to_string() + "  https://getcroc.com/?code=x")));
        assert!(!is_err_line("usage 200 rows"));
        assert!(!is_err_line("all good"));
    }
}

/// 判断日志行是否该按错误标红：带 stderr 标记(\x1F)的，或内容以常见错误词(忽略大小写/行首空格)开头。
/// 部分 CLI（如 Go/urfave-cli 的 croc）把 usage 报错打到 stdout，流标记覆盖不到，需按内容兜底。
pub fn is_err_line(line: &str) -> bool {
    let l = line.strip_prefix('\u{1f}').unwrap_or(line).trim_start();
    if l.is_empty() {
        return false;
    }
    let lower = l.to_ascii_lowercase();
    const PREFIXES: &[&str] = &[
        "error",
        "fatal",
        "panic",
        "incorrect usage",
        "flag provided but not defined",
        "usage:",
        "failed",
        "failure",
        "cannot",
        "unable",
        "refused",
    ];
    PREFIXES.iter().any(|p| lower.starts_with(p))
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
    /// 网络代理设置（加速前缀 + 通用代理），应用于所有受限请求
    #[serde(default, skip_serializing_if = "ProxySettings::is_empty")]
    pub proxy: ProxySettings,
    /// 界面语言：`auto`（跟随系统）/ `zh-CN` / `en`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub locale: String,
    /// Web 管理界面监听设置（空 = 默认回环 + 随机端口）
    #[serde(default, skip_serializing_if = "WebSettings::is_default")]
    pub web: WebSettings,
}

/// Web 管理界面监听设置（F7：端口/绑定可配置）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WebSettings {
    /// 监听地址：空 = 默认回环 127.0.0.1
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bind: String,
    /// 端口：0 = 自动选空闲高位端口
    #[serde(default, skip_serializing_if = "WebSettings::is_zero_port")]
    pub port: u16,
    /// 访问令牌：非回环 peer 必须携带（查询参数 token 或 X-Universal-Token 头）。
    /// 首次启动随机生成并持久化。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
}

impl WebSettings {
    fn is_zero_port(p: &u16) -> bool {
        *p == 0
    }

    pub fn is_default(&self) -> bool {
        self.bind.is_empty() && self.port == 0 && self.token.is_empty()
    }

    /// 生效的监听地址（空 → 回环）。
    pub fn effective_bind(&self) -> &str {
        if self.bind.is_empty() {
            "127.0.0.1"
        } else {
            &self.bind
        }
    }
}

/// 网络代理/加速设置。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ProxySettings {
    /// 加速前缀（重写 GitHub API / 下载 URL），如 "https://gh-proxy.com/"
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub accelerate_prefix: String,
    /// 通用代理（HTTP/SOCKS5），如 "http://127.0.0.1:7890"
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub http_proxy: String,
}

impl ProxySettings {
    pub fn is_empty(&self) -> bool {
        self.accelerate_prefix.is_empty() && self.http_proxy.is_empty()
    }
}

impl ShellConfig {
    pub fn load(path: &PathBuf) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Self = serde_json::from_str(&raw)?;
        Ok(cfg)
    }
}