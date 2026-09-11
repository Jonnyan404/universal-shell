use std::time::{Duration, Instant};

use reqwest::blocking::Client;

/// 对单个 URL 发 HEAD 请求并返回往返毫秒数。
/// 任何网络/超时错误返回 None。
pub fn ping_url(url: &str) -> Option<u64> {
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    let start = Instant::now();
    let _resp = client
        .head(url)
        .header("User-Agent", "universal-shell")
        .send()
        .ok()?;
    Some(start.elapsed().as_millis() as u64)
}

/// 通过代理发送请求到测试端点，返回往返毫秒数。
/// 用 HEAD https://api.github.com/ 做探测（轻量 + 无需认证返回 404 即可）。
pub fn ping_proxy(proxy_url: &str) -> Option<u64> {
    let proxy = reqwest::Proxy::all(proxy_url).ok()?;
    let client = Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    let start = Instant::now();
    let _resp = client
        .head("https://api.github.com/")
        .header("User-Agent", "universal-shell")
        .send()
        .ok()?;
    // 能连上就说明代理可用
    Some(start.elapsed().as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_invalid_returns_none() {
        assert!(ping_url("http://192.0.2.1:1").is_none());
    }

    #[test]
    fn ping_invalid_proxy_returns_none() {
        assert!(ping_proxy("http://192.0.2.1:1").is_none());
    }
}
