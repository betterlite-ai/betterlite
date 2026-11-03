#![cfg(test)]

use super::*;
#[test]
fn proxy_request_carries_http_context() {
    let data = ProxyRequestSpanData {
        http_method: "GET".to_owned(),
        route: "/health".to_owned(),
        url_path: None,
        status_code: Some(200),
        identity: None,
    };
    assert_eq!(data.status_code, Some(200));
}
