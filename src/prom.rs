//! Prometheus 클라이언트 — C 컴파일러/TLS 의존 없이 순수 tokio TCP 로 HTTP/1.0 GET.
//! Prometheus 는 평문 HTTP 이므로 TLS 불필요. HTTP/1.0 + Connection close 로 chunked 회피.

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
    let host = base.trim_start_matches("http://").trim_end_matches('/');
    let fut = async {
        let mut stream = TcpStream::connect(host).await?;
        let req = format!(
            "GET {} HTTP/1.0\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
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
    // 헤더/바디 분리
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .ok_or_else(|| anyhow!("malformed http response"))?;
    Ok(body.to_string())
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
