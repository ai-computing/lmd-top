//! Prometheus 클라이언트 — C 컴파일러/TLS 의존 없이 순수 tokio TCP 로 HTTP/1.1 GET.
//! Connection: close + chunked 디코드로 1.1 전용 프록시 뒤에서도 동작. 전송 오류는 1회 재시도.
//! TLS(https)는 의도적으로 미지원(glibc-only ethos) — port-forward 또는 평문 endpoint 사용.

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

/// promql 한 건 질의 → 결과 벡터. 실패해도 빈 벡터가 아니라 Err 로 전파(상위에서 graceful 처리).
pub async fn query(base: &str, promql: &str) -> Result<Vec<Series>> {
    let path = format!("/api/v1/query?query={}", urlencode(promql));
    let body = http_get(base, &path).await?;
    parse(&body)
}

/// 여러 promql 을 동시(병렬) 조회 — 순차 라운드트립 제거. 결과는 입력 순서 유지.
/// tokio JoinSet 사용(추가 의존성 없음). 각 태스크가 base/promql 을 소유(clone).
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

/// 라벨의 값 목록 조회(`/api/v1/label/<label>/values`). label="__name__" → 전체 메트릭 이름,
/// label="job" → 스크레이프 잡(=exporter) 목록. doctor(전수조사)용.
pub async fn label_values(base: &str, label: &str) -> Result<Vec<String>> {
    let body = http_get(base, &format!("/api/v1/label/{}/values", label)).await?;
    let v: serde_json::Value = serde_json::from_str(&body)?;
    if v["status"] != "success" {
        return Err(anyhow!("prometheus status != success"));
    }
    Ok(v["data"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default())
}

async fn http_get(base: &str, path: &str) -> Result<String> {
    // TLS 는 의도적으로 미지원(순수 Rust·glibc-only). https 면 명확히 안내.
    if base.starts_with("https://") {
        return Err(anyhow!("HTTPS Prometheus not supported (no TLS) — use plain-HTTP endpoint or `kubectl port-forward`"));
    }
    let host = base.trim_start_matches("http://").trim_end_matches('/');
    // 전송 오류(연결/리셋)는 1회 재시도. 상태 오류는 즉시 전파.
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
        // HTTP/1.1 + Connection: close — 1.1 전용 프록시 뒤에서도 동작. chunked 는 아래에서 디코드.
        let req = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nUser-Agent: lmd-top\r\nConnection: close\r\n\r\n",
            path, host
        );
        stream.write_all(req.as_bytes()).await?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await?;
        Ok::<_, anyhow::Error>(String::from_utf8_lossy(&buf).into_owned())
    };
    let raw = timeout(Duration::from_secs(6), fut).await.map_err(|_| anyhow!("prometheus timeout"))??;
    let (head, body) = raw.split_once("\r\n\r\n").ok_or_else(|| anyhow!("malformed http response"))?;
    // 상태줄 체크(4xx/5xx 는 명확히).
    if let Some(status) = head.lines().next() {
        if let Some(code) = status.split_whitespace().nth(1) {
            if code.starts_with('4') || code.starts_with('5') {
                return Err(anyhow!("prometheus HTTP {}", code));
            }
        }
    }
    // Transfer-Encoding: chunked 면 디코드.
    if head.to_lowercase().contains("transfer-encoding: chunked") {
        Ok(dechunk(body))
    } else {
        Ok(body.to_string())
    }
}

/// HTTP chunked transfer 디코드. 청크 크기는 바이트 오프셋이라 char 경계에서 자를 수 있음 →
/// `get(..size)` 로 안전하게 슬라이스(경계 아니면 남은 전부 취하고 종료, 패닉 방지).
fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size_line, after)) = rest.split_once("\r\n") {
        let size = usize::from_str_radix(size_line.trim().split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        match after.get(..size) {
            Some(chunk) => {
                out.push_str(chunk);
                rest = after.get(size..).unwrap_or("").strip_prefix("\r\n").unwrap_or_else(|| after.get(size..).unwrap_or(""));
            }
            // 남은 바이트 부족 or char 경계 아님 → 남은 전부 취하고 종료.
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
        // "Wiki" + "pedia" chunked → "Wikipedia" (경계·CRLF 처리)
        let body = "4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(dechunk(body), "Wikipedia");
    }

    #[test]
    fn dechunk_single_json() {
        let json = r#"{"status":"success"}"#;
        let body = format!("{:x}\r\n{}\r\n0\r\n\r\n", json.len(), json);
        assert_eq!(dechunk(&body), json);
        // 디코드 결과가 다시 파싱되는지.
        assert!(parse(&dechunk(&body)).is_ok());
    }

    #[test]
    fn dechunk_truncated_no_panic() {
        // 선언 크기보다 바디가 짧음 → 패닉 없이 남은 것만.
        let body = "FF\r\nshort";
        let _ = dechunk(body); // 패닉 안 하면 통과
    }

    #[test]
    fn dechunk_non_ascii_boundary_no_panic() {
        // 멀티바이트(한글)가 청크 크기와 어긋나도 패닉 없이 처리.
        let body = "1\r\n가\r\n0\r\n\r\n"; // size=1 인데 '가'는 3바이트 → get(..1)=None
        let _ = dechunk(body);
    }

    #[test]
    fn dechunk_size_zero_terminates() {
        assert_eq!(dechunk("0\r\n\r\n"), "");
    }
}
