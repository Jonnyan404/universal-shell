//! 运行时 locale：解析系统语言 + 手动覆盖，并同步到 rust_i18n 的全局状态。
//!
//! 各层（shared / Tauri 后端 / egui）共用 `set`/`apply`，保证当前语言一致。
//! 单一翻译源为 `shared/locales/*.yml`。

/// 默认语言（跟随系统解析失败时的回退）。
pub const DEFAULT_LOCALE: &str = "zh-CN";

/// 支持的语言列表（与 `shared/locales/*.yml` 文件一一对应）。
pub const LOCALES: &[&str] = &["zh-CN", "en"];

/// 把系统 locale 提示归一化为受支持的 locale，找不到回退到默认中文。
pub fn normalize_system_hint(hint: &str) -> &'static str {
    let h = hint.to_ascii_lowercase();
    if h.starts_with("zh") {
        "zh-CN"
    } else if h.starts_with("en") {
        "en"
    } else {
        DEFAULT_LOCALE
    }
}

/// 直接设置当前语言（`locale` 需是 `LOCALES` 中的有效值）。
pub fn set(locale: &str) {
    rust_i18n::set_locale(locale);
}

/// 根据手动覆盖（`None` = 跟随系统）与系统提示解析最终语言并应用，返回所采用的语言。
pub fn apply(override_locale: Option<&str>, system_hint: &str) -> &'static str {
    let final_locale = match override_locale {
        Some(l) => LOCALES
            .iter()
            .find(|x| **x == l)
            .copied()
            .unwrap_or_else(|| normalize_system_hint(system_hint)),
        None => normalize_system_hint(system_hint),
    };
    rust_i18n::set_locale(final_locale);
    final_locale
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_i18n::t;

    #[test]
    fn resolve_normalizes_system_hint() {
        assert_eq!(normalize_system_hint("zh-Hans-CN"), "zh-CN");
        assert_eq!(normalize_system_hint("en-US"), "en");
        assert_eq!(normalize_system_hint("ja-JP"), DEFAULT_LOCALE);
    }

    #[test]
    fn apply_switches_runtime_locale() {
        assert_eq!(apply(None, "en-US"), "en");
        assert_eq!(t!("app.name"), "Universal Shell");
        assert_eq!(apply(Some("zh-CN"), "en-US"), "zh-CN");
        assert_eq!(t!("lang.title"), "切换语言");
        // 手动指定非法语言回退到系统
        assert_eq!(apply(Some("xx"), "en-US"), "en");
    }
}
