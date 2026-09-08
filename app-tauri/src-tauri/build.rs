//! Tauri 构建脚本：旧版内嵌 UI 已废弃，窗口内容由 rust 启动后
//! navigate 到 web-server（frontend/ 单一起源）的 loopback 页面，
//! `app-tauri/src/` 仅保留一个静态启动占位页，无需生成 locale JSON。
//! locale 单一翻译源在 shared/locales/*.yml，转换由 web-server 构建完成。

fn main() {
    tauri_build::build()
}