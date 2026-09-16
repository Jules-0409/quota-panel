use std::time::Duration;

/// 所有出站请求统一带的 User-Agent：如实报自己的名字和版本。
///
/// 不要伪造成 Cursor / Devin / Factory 的客户端。2026-09-17 实测过：
/// `cursor.com/api/usage-summary` 在伪造 UA、诚实 UA、完全不发 UA 三种情况下都返回 200
/// 且数据完全相同；Devin 的 `GetUserStatus` 也一样，两种请求解析出来的字段和数值
/// 逐一相同（响应体大小差 56 倍，但差的全是本项目不读的部分）。
/// 也就是说伪装没有任何功能收益，只是白白违反对方的服务条款。
pub(crate) const USER_AGENT: &str = concat!("quota-panel/", env!("CARGO_PKG_VERSION"));

/// 最多尝试次数（首次请求 + 2 次重试）。
const MAX_ATTEMPTS: u32 = 3;

/// 每次重试前的等待时间，指数退避：300ms、900ms。
const BACKOFF_MS: [u64; 2] = [300, 900];

/// 发送 HTTP 请求，对**网络错误、5xx、429** 做有限次重试。
///
/// 4xx（除 429）不重试：401/403 意味着登录态已失效，重试没有意义，
/// 直接把响应交回调用方去生成对应的错误文案。
///
/// 重试完仍失败时：传输层错误返回 `Err(reqwest::Error)`；
/// 可重试的 HTTP 状态码返回最后一次的 `Ok(Response)`，由调用方按状态码报错。
///
/// `RequestBuilder` 是一次性的（`send()` 会消费掉它），因此这里接收一个
/// 每次重新构造请求的闭包，请求本身基于共享的 `reqwest::Client` 构造。
pub(crate) async fn send_with_retry<F>(
    build_request: F,
) -> Result<reqwest::Response, reqwest::Error>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;
        let is_last = attempt >= MAX_ATTEMPTS;

        match build_request().send().await {
            Ok(resp) => {
                let status = resp.status();
                let retryable =
                    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
                if !retryable || is_last {
                    return Ok(resp);
                }
            }
            Err(e) => {
                let retryable = e.is_connect() || e.is_timeout() || e.is_request() || e.is_body();
                if !retryable || is_last {
                    return Err(e);
                }
            }
        }

        let delay_ms = BACKOFF_MS[(attempt - 1) as usize];
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}
