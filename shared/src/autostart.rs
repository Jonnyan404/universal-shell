//! 开机启动(auto-launch)。
//! 注意：auto-launch 把「当前可执行文件」注册为登录项，
//! 对「被管的第三方程序」做开机启动，简单且跨平台的方式是给它生成
//! 平台对应的 LaunchAgent/systemd/Startup 条目。为保持跨平台简单，
//! 这里统一用 AppName 区分，一条配置管一个程序。

use anyhow::anyhow;
use auto_launch::{AutoLaunch, AutoLaunchBuilder};

pub struct AutoStart {
    apps: std::collections::BTreeMap<String, AutoLaunch>,
}

impl Default for AutoStart {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoStart {
    pub fn new() -> Self {
        let legacy_path = std::env::current_exe().unwrap_or_default();
        let legacy_path = legacy_path.to_string_lossy().into_owned();
        let legacy = AutoLaunchBuilder::new()
            .set_app_name("cloud-clipboard-go")
            .set_app_path(&legacy_path)
            .build();
        let legacy = legacy
            .map(|a| {
                let mut m = std::collections::BTreeMap::new();
                m.insert("cloud-clipboard-go".into(), a);
                m
            })
            .unwrap_or_default();
        Self { apps: legacy }
    }

    /// 注册/解除某程序的开机启动
    pub fn set_enabled(&mut self, id: &str, path: &std::path::Path, enabled: bool) -> anyhow::Result<()> {
let app_path = path.to_string_lossy().into_owned();
        let mut builder = AutoLaunchBuilder::new();
        builder.set_app_name(id);
        builder.set_app_path(&app_path);
        if std::env::consts::OS == "windows" {
            builder.set_args(&[""]);
        }
        if std::env::consts::OS == "macos" {
            builder.set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent);
        }
        let app = builder
            .build()
            .map_err(|e| anyhow!("auto-launch 构建失败: {e}"))?;
        let res = if enabled {
            app.enable()
        } else {
            app.disable()
        };
        if let Err(e) = res {
            return Err(anyhow!("设置开机启动 {enabled} 失败: {e}"));
        }
        self.apps.insert(id.to_string(), app);
        Ok(())
    }

    #[allow(dead_code)]
    pub fn is_enabled(&self, id: &str) -> bool {
        self.apps
            .get(id)
            .map(|a| a.is_enabled().unwrap_or(false))
            .unwrap_or(false)
    }
}