#![cfg(test)]

use rstest::rstest;

use super::{SyntheticMcpRequest, build_synthetic_mcp_request};

fn pairs(values: &[(&str, &str)]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn request(
    raw_headers: Option<&[(String, String)]>,
    client_ip: Option<&str>,
) -> SyntheticMcpRequest {
    build_synthetic_mcp_request("/srv/mcp", raw_headers, client_ip, &[])
}

fn header_names(request: &SyntheticMcpRequest) -> Vec<String> {
    request
        .headers
        .iter()
        .map(|(name, _)| String::from_utf8(name.clone()).unwrap())
        .collect()
}

#[test]
fn build_synthetic_mcp_request_fills_the_fixed_scope() {
    let built = request(None, None);
    assert_eq!(built.method, "POST");
    assert_eq!(built.path, "/srv/mcp");
    assert_eq!(built.scheme, "http");
    assert_eq!(built.server, ("127.0.0.1".to_owned(), 4000));
    assert!(built.query_string.is_empty());
    assert_eq!(built.root_path, "");
    assert_eq!(built.client, None);
}

#[test]
fn build_synthetic_mcp_request_starts_with_the_json_content_type() {
    let built = request(None, None);
    assert_eq!(
        built.headers,
        vec![(b"content-type".to_vec(), b"application/json".to_vec())]
    );
}

#[rstest]
#[case::content_length("Content-Length", "12")]
#[case::transfer_encoding("Transfer-Encoding", "chunked")]
#[case::connection("Connection", "keep-alive")]
#[case::keep_alive("Keep-Alive", "timeout=5")]
#[case::upgrade("Upgrade", "h2c")]
#[case::te("TE", "trailers")]
#[case::trailer("Trailer", "Expires")]
#[case::content_type("Content-Type", "text/plain")]
#[case::host("Host", "example.test")]
#[case::forwarded_for("X-Forwarded-For", "1.2.3.4")]
fn build_synthetic_mcp_request_drops_an_excluded_header(#[case] name: &str, #[case] value: &str) {
    let raw = pairs(&[(name, value), ("X-Kept", "kept")]);
    let built = request(Some(&raw), None);
    assert_eq!(header_names(&built), vec!["content-type", "x-kept"]);
}

#[test]
fn build_synthetic_mcp_request_matches_the_excluded_names_case_insensitively() {
    let raw = pairs(&[("HOST", "example.test"), ("X-Forwarded-For", "1.2.3.4")]);
    let built = request(Some(&raw), None);
    assert_eq!(header_names(&built), vec!["content-type"]);
}

#[test]
fn build_synthetic_mcp_request_keeps_the_other_headers_in_order() {
    let raw = pairs(&[
        ("Authorization", "Bearer t"),
        ("x-token", "v"),
        ("X-Mcp-Servers", "srv"),
    ]);
    let built = request(Some(&raw), None);
    assert_eq!(
        built.headers,
        vec![
            (b"content-type".to_vec(), b"application/json".to_vec()),
            (b"authorization".to_vec(), b"Bearer t".to_vec()),
            (b"x-token".to_vec(), b"v".to_vec()),
            (b"x-mcp-servers".to_vec(), b"srv".to_vec()),
        ]
    );
}

#[test]
fn build_synthetic_mcp_request_keeps_a_repeated_header_name() {
    let raw = pairs(&[("X-Dup", "one"), ("X-Dup", "two")]);
    let built = request(Some(&raw), None);
    assert_eq!(
        built.headers,
        vec![
            (b"content-type".to_vec(), b"application/json".to_vec()),
            (b"x-dup".to_vec(), b"one".to_vec()),
            (b"x-dup".to_vec(), b"two".to_vec()),
        ]
    );
}

#[test]
fn build_synthetic_mcp_request_drops_the_extra_excluded_names() {
    let raw = pairs(&[
        ("X-Mcp-Auth", "secret"),
        ("X-Secret", "secret"),
        ("X-Kept", "kept"),
    ]);
    let extra = vec!["x-mcp-auth".to_owned(), "X-SECRET".to_owned()];
    let built = build_synthetic_mcp_request("/srv/mcp", Some(&raw), None, &extra);
    assert_eq!(header_names(&built), vec!["content-type", "x-kept"]);
}

#[test]
fn build_synthetic_mcp_request_adds_the_forwarded_for_and_the_client() {
    let raw = pairs(&[("X-Kept", "kept")]);
    let built = request(Some(&raw), Some("9.9.9.9"));
    assert_eq!(
        built.headers,
        vec![
            (b"content-type".to_vec(), b"application/json".to_vec()),
            (b"x-kept".to_vec(), b"kept".to_vec()),
            (b"x-forwarded-for".to_vec(), b"9.9.9.9".to_vec()),
        ]
    );
    assert_eq!(built.client, Some(("9.9.9.9".to_owned(), 0)));
}

#[test]
fn build_synthetic_mcp_request_encodes_a_value_as_utf8() {
    let raw = pairs(&[("X-Text", "héllo ✓")]);
    let built = request(Some(&raw), Some("9.9.9.9"));
    assert_eq!(built.headers[1].1, "héllo ✓".as_bytes());
    assert_eq!(built.headers[2].1, b"9.9.9.9".to_vec());
}

#[test]
fn build_synthetic_mcp_request_encodes_a_name_as_latin1() {
    let raw = pairs(&[("X-É", "v"), ("X-€", "v")]);
    let built = request(Some(&raw), None);
    assert_eq!(built.headers[1].0, vec![b'x', b'-', 0xE9]);
    assert_eq!(built.headers[2].0, vec![b'x', b'-', b'?']);
}
