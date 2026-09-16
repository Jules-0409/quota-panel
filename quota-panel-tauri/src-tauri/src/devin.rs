use crate::credentials::load_devin_token;
use crate::models::DevinQuota;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

// Minimal Protobuf encoder / decoder
fn encode_varint(mut val: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    while val > 127 {
        buf.push(((val & 0x7f) | 0x80) as u8);
        val >>= 7;
    }
    buf.push(val as u8);
    buf
}

fn encode_tag(field: u32, wire: u8) -> Vec<u8> {
    encode_varint(((field as u64) << 3) | (wire as u64))
}

fn encode_string(field: u32, val: &str) -> Vec<u8> {
    let b = val.as_bytes();
    let mut out = encode_tag(field, 2);
    out.extend(encode_varint(b.len() as u64));
    out.extend_from_slice(b);
    out
}

fn encode_message(field: u32, inner: &[u8]) -> Vec<u8> {
    let mut out = encode_tag(field, 2);
    out.extend(encode_varint(inner.len() as u64));
    out.extend_from_slice(inner);
    out
}

fn decode_varint(buf: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0;
    while pos < buf.len() {
        let byte = buf[pos];
        pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        shift += 7;
        if (byte & 0x80) == 0 {
            return Some((result, pos));
        }
        if shift > 64 {
            return None;
        }
    }
    None
}

#[derive(Debug, Clone)]
enum PbValue<'a> {
    Varint(u64),
    LengthDelimited(&'a [u8]),
}

#[derive(Debug, Clone)]
struct PbField<'a> {
    field: u32,
    value: PbValue<'a>,
}

fn parse_proto(buf: &[u8]) -> Vec<PbField<'_>> {
    let mut fields = Vec::new();
    let mut pos = 0;

    while pos < buf.len() {
        let (tag, next_pos) = match decode_varint(buf, pos) {
            Some(res) => res,
            None => break,
        };
        pos = next_pos;

        let field = (tag >> 3) as u32;
        let wire = (tag & 0x07) as u8;
        if field == 0 {
            break;
        }

        match wire {
            0 => {
                if let Some((v, p)) = decode_varint(buf, pos) {
                    pos = p;
                    fields.push(PbField {
                        field,
                        value: PbValue::Varint(v),
                    });
                } else {
                    break;
                }
            }
            2 => {
                if let Some((len, p)) = decode_varint(buf, pos) {
                    let len = len as usize;
                    if p + len <= buf.len() {
                        fields.push(PbField {
                            field,
                            value: PbValue::LengthDelimited(&buf[p..p + len]),
                        });
                        pos = p + len;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            5 => {
                if pos + 4 <= buf.len() {
                    fields.push(PbField {
                        field,
                        value: PbValue::LengthDelimited(&buf[pos..pos + 4]),
                    });
                    pos += 4;
                } else {
                    break;
                }
            }
            1 => {
                if pos + 8 <= buf.len() {
                    fields.push(PbField {
                        field,
                        value: PbValue::LengthDelimited(&buf[pos..pos + 8]),
                    });
                    pos += 8;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }

    fields
}

fn get_varint(fields: &[PbField], target_field: u32) -> Option<u64> {
    fields.iter().find_map(|f| {
        if f.field == target_field {
            if let PbValue::Varint(v) = f.value {
                return Some(v);
            }
        }
        None
    })
}

fn get_sub(fields: &[PbField], target_field: u32) -> Option<Vec<u8>> {
    fields.iter().find_map(|f| {
        if f.field == target_field {
            if let PbValue::LengthDelimited(slice) = f.value {
                return Some(slice.to_vec());
            }
        }
        None
    })
}

fn get_string(fields: &[PbField], target_field: u32) -> Option<String> {
    get_sub(fields, target_field)
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

fn get_timestamp(fields: &[PbField], target_field: u32) -> Option<i64> {
    let sub = get_sub(fields, target_field)?;
    let parsed = parse_proto(&sub);
    get_varint(&parsed, 1).map(|v| v as i64)
}

pub async fn fetch_devin_live(
    client: &reqwest::Client,
    token: &str,
    source: &str,
) -> Result<DevinQuota, String> {
    let mut metadata = Vec::new();
    // 客户端身份如实填写。原来这里硬编码自称 Devin 自家的 "chisel" 客户端、
    // 版本 "0.0.0-dev"、平台 "windows"（在 macOS 上也照发）。
    // 2026-09-17 实测：服务端不校验这些字段，如实填写与谎报解析出的字段和数值完全一致。
    metadata.extend(encode_string(1, "quota-panel"));
    metadata.extend(encode_string(2, env!("CARGO_PKG_VERSION")));
    metadata.extend(encode_string(3, token));
    metadata.extend(encode_string(4, "en"));
    metadata.extend(encode_string(5, std::env::consts::OS));
    metadata.extend(encode_string(7, env!("CARGO_PKG_VERSION")));

    let body = encode_message(1, &metadata);

    let mut headers = HeaderMap::new();
    let auth_val = format!("Basic {}-{}", token, token);
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&auth_val).map_err(|e| e.to_string())?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/proto"));
    headers.insert("connect-protocol-version", HeaderValue::from_static("1"));
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&body.len().to_string()).unwrap(),
    );

    // 网络错误 / 5xx / 429 自动重试（最多 3 次尝试，300ms、900ms 指数退避）。
    // RequestBuilder 是一次性的，重试时要用 headers / body 的克隆重新构造。
    let send_result = crate::http::send_with_retry(|| {
        client
            .post("https://server.codeium.com/exa.seat_management_pb.SeatManagementService/GetUserStatus")
            .headers(headers.clone())
            .body(body.clone())
    })
    .await;

    let resp = send_result.map_err(|e| format!("Devin request failed: {}", e))?;

    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err("Devin 登录态已失效，请重新登录 Devin Desktop".into());
    }
    if !status.is_success() {
        return Err(format!("Devin API returned status {}", status));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read Devin response body: {}", e))?;

    let top = parse_proto(&bytes);
    let user_status_bytes = get_sub(&top, 1).unwrap_or_default();
    let user_status = parse_proto(&user_status_bytes);

    let email = get_string(&user_status, 7);
    let user_id = get_string(&user_status, 36);
    let team_id = get_string(&user_status, 5);

    let plan_status_bytes = get_sub(&user_status, 13).unwrap_or_default();
    let plan_status = parse_proto(&plan_status_bytes);

    let plan_info_bytes = get_sub(&plan_status, 1).unwrap_or_default();
    let plan_info = parse_proto(&plan_info_bytes);
    let plan_name = get_string(&plan_info, 2);

    let daily_remaining_percent = get_varint(&plan_status, 14).map(|v| v as f64);
    let weekly_remaining_percent = get_varint(&plan_status, 15).map(|v| v as f64);
    let overage_balance_micros = get_varint(&plan_status, 16).map(|v| v as i64);
    let daily_reset_at_unix = get_varint(&plan_status, 17).map(|v| v as i64);
    let weekly_reset_at_unix = get_varint(&plan_status, 18).map(|v| v as i64);
    let acu_consumed = get_varint(&plan_status, 19).map(|v| v as i64);
    let acu_limit = get_varint(&plan_status, 20).map(|v| v as i64);

    let plan_start_unix = get_timestamp(&plan_status, 2);
    let plan_end_unix = get_timestamp(&plan_status, 3);

    Ok(DevinQuota {
        ok: true,
        id: "devin".into(),
        name: "Devin".into(),
        plan_name,
        email,
        user_id,
        team_id,
        daily_remaining_percent,
        weekly_remaining_percent,
        daily_reset_at_unix,
        weekly_reset_at_unix,
        overage_balance_micros,
        acu_consumed,
        acu_limit,
        plan_start_unix,
        plan_end_unix,
        source: Some(source.to_string()),
        stale: false,
        stale_reason: None,
        fetched_at: now_millis(),
        error: None,
    })
}

pub async fn fetch_devin(client: &reqwest::Client) -> DevinQuota {
    // load_devin_token 是同步阻塞的（读 credentials.toml、只读打开 state.vscdb），
    // 放到阻塞线程池里，别占住 tokio 的 worker 线程
    let loaded = tokio::task::spawn_blocking(load_devin_token).await;

    let (token, source) = match loaded {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => {
            return DevinQuota {
                ok: false,
                id: "devin".into(),
                name: "Devin".into(),
                error: Some(e),
                fetched_at: now_millis(),
                ..Default::default()
            };
        }
        Err(join_err) => {
            return DevinQuota {
                ok: false,
                id: "devin".into(),
                name: "Devin".into(),
                error: Some(format!("读取 Devin 凭据的任务失败: {}", join_err)),
                fetched_at: now_millis(),
                ..Default::default()
            };
        }
    };

    let live_result = fetch_devin_live(client, &token, &source).await;

    match live_result {
        Ok(quota) => quota,
        Err(e) => DevinQuota {
            ok: false,
            id: "devin".into(),
            name: "Devin".into(),
            source: Some(source),
            error: Some(e),
            fetched_at: now_millis(),
            ..Default::default()
        },
    }
}
