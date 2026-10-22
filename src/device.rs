//! 设备层薄封装：只做 SDK 调用与参数校验，不重试、不自动重连、不抢占。

use anyhow::Result;
use zectrix_sdk::{AudioClip, Device, ImageFormat, OpenOptions};

pub async fn open(target: Option<&str>) -> Result<Device> {
    let target = target.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    Ok(Device::open(OpenOptions {
        target,
        client_name: Some("ZECTRIX Pet 4".into()),
    })
    .await?)
}

pub fn format(name: &str) -> Result<ImageFormat> {
    match name.trim().to_ascii_lowercase().as_str() {
        "mono" => Ok(ImageFormat::Mono),
        "gray4" | "gray-4" => Ok(ImageFormat::Gray4),
        "gray16" | "gray-16" | "" => Ok(ImageFormat::Gray16),
        other => anyhow::bail!("未知灰阶格式 “{other}”，可用 mono / gray4 / gray16"),
    }
}

pub fn format_name(f: ImageFormat) -> &'static str {
    match f {
        ImageFormat::Mono => "mono",
        ImageFormat::Gray4 => "gray4",
        ImageFormat::Gray16 => "gray16",
    }
}

/// SDK 的音频固定为 16 kHz 单声道 PCM16，这里把字节流还原成采样。
pub fn clip(pcm_le: &[u8]) -> AudioClip {
    AudioClip {
        samples: pcm_le
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect(),
    }
}
