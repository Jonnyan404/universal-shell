//! 下载/安装进度事件模型。纯数据结构，供 shared 在安装管线的各阶段回调，
//! 由上层(如 Tauri 命令)转成 UI 进度条。

/// 安装管线阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStage {
    /// 正在下载资产
    Downloading,
    /// 校验 sha256
    Verifying,
    /// 解压/落地
    Extracting,
}

/// 一次进度回调携带的信息。
/// `received`/`total` 仅 Downloading 阶段有效(total 未知时为 0)。
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct DownloadProgress {
    pub stage: DownloadStage,
    pub received: u64,
    pub total: u64,
}

impl DownloadProgress {
    pub fn stage(stage: DownloadStage) -> Self {
        Self {
            stage,
            received: 0,
            total: 0,
        }
    }

    pub fn downloading(received: u64, total: u64) -> Self {
        Self {
            stage: DownloadStage::Downloading,
            received,
            total,
        }
    }

    /// 0.0..=1.0；total 未知(total==0)时返回 None。
    pub fn fraction(&self) -> Option<f64> {
        if self.total == 0 {
            None
        } else {
            Some((self.received as f64 / self.total as f64).clamp(0.0, 1.0))
        }
    }
}
