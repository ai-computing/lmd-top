//! Prometheus client — HTTP/1.1 GET over pure tokio TCP, no C compiler/TLS dependency.
//! Connection: close + chunked decoding so it works behind 1.1-only proxies. Transport errors retried once.
//! TLS (https) intentionally unsupported (glibc-only ethos) — use port-forward or a plaintext endpoint.

use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

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
    let host = base.trim_start_matches("http://").trim_end_matches('/');
    // Transport errors (connect/reset) retried once. Status errors propagate immediately.
    let mut last = anyhow!("unreachable");
    for attempt in 0..2 {
        match http_get_once(host, path).await {
            Ok(b) => return Ok(b),
            Err(e) => {
                last = e;
                if attempt == 0 {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
            }
        }
    }
    Err(last)
}

async fn http_get_once(host: &str, path: &str) -> Result<String> {
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
        Ok::<_, anyhow::Error>(String::from_utf8_lossy(&buf).into_owned())
    };
    let raw = timeout(Duration::from_secs(6), fut)
        .await
        .map_err(|_| anyhow!("prometheus timeout"))??;
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow!("malformed http response"))?;
    // Check the status line (report 4xx/5xx clearly).
    if let Some(status) = head.lines().next() {
        if let Some(code) = status.split_whitespace().nth(1) {
            if code.starts_with('4') || code.starts_with('5') {
                return Err(anyhow!("prometheus HTTP {}", code));
            }
        }
    }
    // Decode if Transfer-Encoding: chunked.
    if head.to_lowercase().contains("transfer-encoding: chunked") {
        Ok(dechunk(body))
    } else {
        Ok(body.to_string())
    }
}

/// Decode HTTP chunked transfer. Chunk sizes are byte offsets, so they can cut across a char boundary →
/// slice safely with `get(..size)` (if not on a boundary, take the rest and stop; avoids panics).
fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size_line, after)) = rest.split_once("\r\n") {
        let size =
            usize::from_str_radix(size_line.trim().split(';').next().unwrap_or("").trim(), 16)
                .unwrap_or(0);
        if size == 0 {
            break;
        }
        match after.get(..size) {
            Some(chunk) => {
                out.push_str(chunk);
                rest = after
                    .get(size..)
                    .unwrap_or("")
                    .strip_prefix("\r\n")
                    .unwrap_or_else(|| after.get(size..).unwrap_or(""));
            }
            // Not enough bytes left or not on a char boundary → take the rest and stop.
            None => {
                out.push_str(after);
                break;
            }
        }
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

    #[test]
    fn dechunk_multi_chunk() {
        // "Wiki" + "pedia" chunked → "Wikipedia" (boundary/CRLF handling)
        let body = "4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(dechunk(body), "Wikipedia");
    }

    #[test]
    fn dechunk_single_json() {
        let json = r#"{"status":"success"}"#;
        let body = format!("{:x}\r\n{}\r\n0\r\n\r\n", json.len(), json);
        assert_eq!(dechunk(&body), json);
        // Decoded output should parse again.
        assert!(parse(&dechunk(&body)).is_ok());
    }

    #[test]
    fn dechunk_truncated_no_panic() {
        // Body shorter than declared size → take what remains without panicking.
        let body = "FF\r\nshort";
        let _ = dechunk(body); // passes if no panic
    }

    #[test]
    fn dechunk_non_ascii_boundary_no_panic() {
        // Handles a multibyte char misaligned with the chunk size without panicking.
        let body = "1\r\n€\r\n0\r\n\r\n"; // size=1 but '€' is 3 bytes → get(..1)=None
        let _ = dechunk(body);
    }

    #[test]
    fn dechunk_size_zero_terminates() {
        assert_eq!(dechunk("0\r\n\r\n"), "");
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
