use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

mod tests;

// LiteLLM's hop-by-hop set plus content-type, host and x-forwarded-for, all lowercased
const EXCLUDED_HEADERS: [&str; 10] = [
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "upgrade",
    "te",
    "trailer",
    "content-type",
    "host",
    "x-forwarded-for",
];

const SYNTHETIC_SERVER: (&str, u16) = ("127.0.0.1", 4000);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticMcpRequest {
    pub method: String,
    pub path: String,
    pub scheme: String,
    pub server: (String, u16),
    pub query_string: Vec<u8>,
    pub root_path: String,
    pub headers: Vec<(Vec<u8>, Vec<u8>)>,
    pub client: Option<(String, u16)>,
}

#[must_use]
pub fn build_synthetic_mcp_request(
    path: &str,
    raw_headers: Option<&[(String, String)]>,
    client_ip: Option<&str>,
    extra_excluded: &[String],
) -> SyntheticMcpRequest {
    let excluded: BTreeSet<String> = EXCLUDED_HEADERS
        .iter()
        .map(|name| (*name).to_owned())
        .chain(extra_excluded.iter().map(|name| name.to_lowercase()))
        .collect();
    let mut headers: Vec<(Vec<u8>, Vec<u8>)> =
        vec![(b"content-type".to_vec(), b"application/json".to_vec())];
    for (name, value) in raw_headers.unwrap_or_default() {
        let lowered = name.to_lowercase();
        if excluded.contains(&lowered) {
            continue;
        }
        headers.push((latin1_bytes(&lowered), value.as_bytes().to_vec()));
    }
    if let Some(ip) = client_ip {
        headers.push((b"x-forwarded-for".to_vec(), ip.as_bytes().to_vec()));
    }
    SyntheticMcpRequest {
        method: "POST".to_owned(),
        path: path.to_owned(),
        scheme: "http".to_owned(),
        server: (SYNTHETIC_SERVER.0.to_owned(), SYNTHETIC_SERVER.1),
        query_string: Vec::new(),
        root_path: String::new(),
        headers,
        client: client_ip.map(|ip| (ip.to_owned(), 0)),
    }
}

fn latin1_bytes(text: &str) -> Vec<u8> {
    text.chars()
        .map(|character| u8::try_from(u32::from(character)).unwrap_or(b'?'))
        .collect()
}
