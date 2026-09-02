//! universal-shell core：配置驱动的「二进制程序管理壳」核心库。
//!
//! 提供给 GUI(egui / Tauri)复用，也可被 CLI 调用。

pub mod autostart;
pub mod checksum;
pub mod config;
pub mod extract;
pub mod github;
pub mod registry;
pub mod registry_sign;
pub mod runner;
pub mod shell_manager;
pub mod version;

pub use config::{AssetRule, ExtractMode, Field, FieldKind, Program, ShellConfig};
pub use github::GitHub;
pub use registry::{
    load_merged_manifests, Manifest, MergedSource, RegistryClient, RegistryState, TemplateIndex,
};
pub use shell_manager::{ProgramStatus, ShellManager, TemplateDiff};

/// 统一的运行状态错误类型复导出
pub type Result<T> = anyhow::Result<T>;