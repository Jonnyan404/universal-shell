//! universal-shell core：配置驱动的「二进制程序管理壳」核心库。
//!
//! 提供给 GUI(egui / Tauri)复用，也可被 CLI 调用。

pub mod autostart;
pub mod builtin;
pub mod checksum;
pub mod config;
pub mod diff;
pub mod events;
pub mod extract;
pub mod github;
pub mod locale;
pub mod ping;
pub mod progress;
pub mod registry;
pub mod registry_sign;
pub mod runner;
pub mod shell_manager;
pub mod shell_update;
pub mod source_http;
pub mod version;

rust_i18n::i18n!("locales");

pub use config::{
    AssetRule, ExtractMode, Field, FieldKind, PresetEntry, Program, ProxySettings, SavedProxy,
    ShellConfig,
};
pub use github::{clear_github_cache, GitHub};
pub use progress::{DownloadProgress, DownloadStage};
pub use registry::{
    load_merged_manifests, load_merged_manifests_cached, Manifest, MergedSource, RegistryClient,
    RegistryState, TemplateIndex,
};
pub use shell_manager::{ProgramStatus, ShellManager, TemplateDiff, TemplateDiffView};
pub use shell_update::{check_shell_update, ShellUpdate, SHELL_REPO};

/// 统一的运行状态错误类型复导出
pub type Result<T> = anyhow::Result<T>;

/// 无依赖短 ID：时间戳 + 线程局部计数器混合，用于运行时新建的预置/代理条目
pub fn short_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    format!("{:x}{:x}", ms, SEQ.fetch_add(1, Ordering::Relaxed) + 97)
}