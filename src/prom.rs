//! Prometheus client — HTTP/1.1 GET over pure tokio TCP, no C compiler/TLS dependency.
//! Connection: close + chunked decoding so it works behind 1.1-only proxies. Transport errors retried once.
//! TLS (https) intentionally unsupported (glibc-only ethos) — use port-forward or a plaintext endpoint.

use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

/// Circuit breaker — set once a reachability probe fails, cleared when one succeeds.
/// While open, queries fail instantly instead of each burning a 6 s timeout. Without this
/// a blackholed Prometheus made one collect take 92 s (BUG-01), freezing startup for that long.
static PROM_DOWN: AtomicBool = AtomicBool::new(false);

/// Record probe outcome (called by collect's `vector(1)` probe). `up=false` opens the circuit.
pub fn set_reachable(up: bool) {
    PROM_DOWN.store(!up, Ordering::Relaxed);
}

/// Is the circuit open (Prometheus known-unreachable this cycle)?
pub fn is_down() -> bool {
    PROM_DOWN.load(Ordering::Relaxed)
}

/// Reachability probe — short timeout, no retry, ignores the circuit (it is what re-closes it).
pub async fn probe(base: &str) -> bool {
    let path = format!("/api/v1/query?query={}", urlencode("vector(1)"));
    let host = base.trim_start_matches("http://").trim_end_matches('/');
    let up = !base.starts_with("https://")
        && matches!(http_get_once(host, &path, Duration::from_secs(2)).await, Ok(b) if parse(&b).is_ok());
    set_reachable(up);
    up
}

#[derive(Debug, Clone)]
pub struct Series {
    pub labels: BTreeMap<String, String>,
    pub value: f64,
}

impl Series {
    pub fn l(&self, k: &str) -> &str {
        self.labels.get(k).map(|s| s.as_str()).unwrap_or("")
    }
}

/// Query a single promql expression → result vector. Failures propagate as Err, not an empty vector (caller handles gracefully).
pub async fn query(base: &str, promql: &str) -> Result<Vec<Series>> {
    let path = format!("/api/v1/query?query={}", urlencode(promql));
    let body = http_get(base, &path).await?;
    parse(&body)
}

/// Query multiple promql expressions concurrently (in parallel) — removes sequential round-trips. Results keep input order.
/// Uses tokio JoinSet (no extra dependency). Each task owns its base/promql (clone).
pub async fn query_all(base: &str, qs: &[&str]) -> Vec<Result<Vec<Series>>> {
    let mut set = tokio::task::JoinSet::new();
    for (i, q) in qs.iter().enumerate() {
        let base = base.to_string();
        let q = q.to_string();
        set.spawn(async move { (i, query(&base, &q).await) });
    }
    let mut out: Vec<Result<Vec<Series>>> = (0..qs.len()).map(|_| Ok(Vec::new())).collect();
    while let Some(joined) = set.join_next().await {
        if let Ok((i, r)) = joined {
            out[i] = r;
        }
    }
    out
}

/// Fetch value list for a label (`/api/v1/label/<label>/values`). label="__name__" → all metric names,
/// label="job" → scrape jobs (= exporters). Used by doctor (full survey).
pub async fn label_values(base: &str, label: &str) -> Result<Vec<String>> {
    let body = http_get(base, &format!("/api/v1/label/{}/values", label)).await?;
    let v: serde_json::Value = serde_json::from_str(&body)?;
    if v["status"] != "success" {
        return Err(anyhow!("prometheus status != success"));
    }
    Ok(v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

async fn http_get(base: &str, path: &str) -> Result<String> {
    // TLS intentionally unsupported (pure Rust, glibc-only). Give a clear message for https.
    if base.starts_with("https://") {
        return Err(anyhow!("HTTPS Prometheus not supported (no TLS) — use plain-HTTP endpoint or `kubectl port-forward`"));
    }
    // Circuit open → do not touch the network at all (see PROM_DOWN).
    if is_down() {
        return Err(anyhow!("prometheus unreachable (probe failed this cycle)"));
    }
    let host = base.trim_start_matches("http://").trim_end_matches('/');
    // Transport errors (connect/reset) are retried once. Status errors and timeouts propagate
    // immediately — retrying a 6 s timeout only doubles the wait, and a 4xx/5xx will repeat (BUG-11).
    let mut last = anyhow!("unreachable");
    for attempt in 0..2 {
        match http_get_once(host, path, Duration::from_secs(6)).await {
            Ok(b) => return Ok(b),
            Err(e) => {
                let retry = attempt == 0 && is_retryable(&e);
                last = e;
                if !retry {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        }
    }
    Err(last)
}

/// Only transport-level failures are worth a second attempt. An HTTP status error is
/// deterministic, and a timeout already consumed its full budget.
fn is_retryable(e: &anyhow::Error) -> bool {
    let m = e.to_string();
    !(m.contains("prometheus HTTP") || m.contains("timeout") || m.contains("unreachable"))
}

async fn http_get_once(host: &str, path: &str, budget: Duration) -> Result<String> {
    let fut = async {
        let mut stream = TcpStream::connect(host).await?;
        // HTTP/1.1 + Connection: close — works behind 1.1-only proxies. Chunked decoded below.
        let req = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nUser-Agent: lmd-top\r\nConnection: close\r\n\r\n",
            path, host
        );
        stream.write_all(req.as_bytes()).await?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await?;
        Ok::<Vec<u8>, anyhow::Error>(buf)
    };
    // Response stays bytes until the body is fully de-chunked. Decoding chunk-size *byte* offsets
    // on a lossy String shifts every offset once a chunk splits a multi-byte char (BUG-13).
    let raw = timeout(budget, fut)
        .await
        .map_err(|_| anyhow!("prometheus timeout"))??;
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("malformed http response"))?;
    let (head, body) = (&raw[..sep], &raw[sep + 4..]);
    let head = String::from_utf8_lossy(head); // headers are ASCII by spec
    // Check the status line (report 4xx/5xx clearly).
    if let Some(status) = head.lines().next() {
        if let Some(code) = status.split_whitespace().nth(1) {
            if code.starts_with('4') || code.starts_with('5') {
                return Err(anyhow!("prometheus HTTP {}", code));
            }
        }
    }
    // Decode if Transfer-Encoding: chunked.
    let body = if head.to_lowercase().contains("transfer-encoding: chunked") {
        dechunk(body)
    } else {
        body.to_vec()
    };
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Decode HTTP chunked transfer, on bytes. A chunk boundary may fall in the middle of a
/// multi-byte character, so the join has to happen before any UTF-8 interpretation —
/// doing it on a `&str` corrupts the body (and any non-ASCII label with it).
/// A truncated final chunk yields whatever bytes remain (no panic, no header bleed).
fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut rest = body;
    while let Some(nl) = rest.windows(2).position(|w| w == b"\r\n") {
        let size_line = String::from_utf8_lossy(&rest[..nl]);
        let size =
            usize::from_str_radix(size_line.trim().split(';').next().unwrap_or("").trim(), 16)
                .unwrap_or(0);
        let after = &rest[nl + 2..];
        if size == 0 {
            break;
        }
        if size >= after.len() {
            // Declared size exceeds what arrived → take the remainder and stop.
            out.extend_from_slice(after);
            break;
        }
        out.extend_from_slice(&after[..size]);
        rest = after[size..].strip_prefix(b"\r\n").unwrap_or(&after[size..]);
    }
    out
}

fn parse(body: &str) -> Result<Vec<Series>> {
    let v: serde_json::Value = serde_json::from_str(body)?;
    if v["status"] != "success" {
        return Err(anyhow!("prometheus status != success"));
    }
    let mut out = Vec::new();
    if let Some(arr) = v["data"]["result"].as_array() {
        for item in arr {
            let mut labels = BTreeMap::new();
            if let Some(m) = item["metric"].as_object() {
                for (k, val) in m {
                    if let Some(s) = val.as_str() {
                        labels.insert(k.clone(), s.to_string());
                    }
                }
            }
            // value: [ <ts>, "<num>" ]
            let value = item["value"][1]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(f64::NAN);
            out.push(Series { labels, value });
        }
    }
    Ok(out)
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(body: &[u8]) -> String {
        String::from_utf8_lossy(&dechunk(body)).into_owned()
    }

    #[test]
    fn dechunk_multi_chunk() {
        // "Wiki" + "pedia" chunked → "Wikipedia" (boundary/CRLF handling)
        assert_eq!(dec(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n"), "Wikipedia");
    }

    #[test]
    fn dechunk_single_json() {
        let json = r#"{"status":"success"}"#;
        let body = format!("{:x}\r\n{}\r\n0\r\n\r\n", json.len(), json);
        assert_eq!(dec(body.as_bytes()), json);
        // Decoded output should parse again.
        assert!(parse(&dec(body.as_bytes())).is_ok());
    }

    #[test]
    fn dechunk_truncated_no_panic() {
        // Body shorter than declared size → take what remains without panicking.
        assert_eq!(dec(b"FF\r\nshort"), "short");
    }

    #[test]
    fn dechunk_splits_multibyte_char_exactly() {
        // BUG-13 회귀: 서버가 '€'(E2 82 AC) 중간에서 청크를 끊어도 문자가 온전히 복원돼야 한다.
        // (문자열 위 바이트 오프셋 슬라이싱은 여기서 U+FFFD 삽입 → 오프셋 어긋남 → 본문 손상.)
        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(b"2\r\n");
        body.extend_from_slice(&[0xE2, 0x82]);
        body.extend_from_slice(b"\r\n1\r\n");
        body.extend_from_slice(&[0xAC]);
        body.extend_from_slice(b"\r\n0\r\n\r\n");
        assert_eq!(dec(&body), "€");
    }

    #[test]
    fn dechunk_non_ascii_json_across_chunks() {
        // 한국어 라벨이 청크 경계에 걸려도 JSON 이 그대로 파싱돼야 한다(FAULT-04).
        let json = r#"{"status":"success","data":{"result":[{"metric":{"pod":"모델-서버"},"value":[1.0,"7"]}]}}"#;
        let b = json.as_bytes();
        let cut = 60; // '모' 바이트열 중간을 가로지르도록 자름
        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(format!("{:x}\r\n", cut).as_bytes());
        body.extend_from_slice(&b[..cut]);
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("{:x}\r\n", b.len() - cut).as_bytes());
        body.extend_from_slice(&b[cut..]);
        body.extend_from_slice(b"\r\n0\r\n\r\n");
        let out = dec(&body);
        assert_eq!(out, json);
        let v = parse(&out).expect("decoded body still parses");
        assert_eq!(v[0].l("pod"), "모델-서버");
    }

    #[test]
    fn dechunk_size_zero_terminates() {
        assert_eq!(dec(b"0\r\n\r\n"), "");
    }

    #[test]
    fn retry_only_transport_errors() {
        // BUG-11 회귀: 상태 오류/타임아웃은 재시도 대상이 아니다.
        assert!(!is_retryable(&anyhow!("prometheus HTTP 400")));
        assert!(!is_retryable(&anyhow!("prometheus HTTP 503")));
        assert!(!is_retryable(&anyhow!("prometheus timeout")));
        assert!(!is_retryable(&anyhow!(
            "prometheus unreachable (probe failed this cycle)"
        )));
        // 전송 오류는 한 번 더 시도한다.
        assert!(is_retryable(&anyhow!("connection reset by peer")));
    }

    #[test]
    fn circuit_breaker_short_circuits_queries() {
        // BUG-01 회귀: 프로브 실패 후 쿼리는 네트워크를 건드리지 않고 즉시 오류로 돌아온다.
        set_reachable(false);
        assert!(is_down());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let t0 = std::time::Instant::now();
        let r = rt.block_on(query("192.0.2.7:9", "vector(1)"));
        assert!(r.is_err());
        assert!(
            t0.elapsed() < std::time::Duration::from_millis(200),
            "circuit-open query must not wait on the network: {:?}",
            t0.elapsed()
        );
        set_reachable(true);
        assert!(!is_down());
    }

    #[test]
    fn parse_success_and_labels() {
        let body = r#"{"status":"success","data":{"result":[
            {"metric":{"service":"koni","le":"1"},"value":[1.0,"3.5"]}
        ]}}"#;
        let v = parse(body).expect("ok");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].value, 3.5);
        assert_eq!(v[0].l("service"), "koni");
    }

    #[test]
    fn parse_plus_inf_and_missing() {
        // Prometheus returns "+Inf"/"NaN" strings when histogram_quantile has no data.
        let body = r#"{"status":"success","data":{"result":[
            {"metric":{},"value":[1.0,"+Inf"]},
            {"metric":{},"value":[1.0,"NaN"]}
        ]}}"#;
        let v = parse(body).expect("ok");
        assert!(v[0].value.is_infinite());
        assert!(v[1].value.is_nan());
    }

    #[test]
    fn parse_non_success_is_err() {
        assert!(parse(r#"{"status":"error","error":"boom"}"#).is_err());
        assert!(parse("not json").is_err());
    }
}
