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

#[test]
fn proxy_request_retains_identity() {
    let data: ProxyRequestSpanData = serde_json::from_str(r#"{"http_method":"POST","route":"/chat","url_path":"/chat","status_code":201,"identity":{"call_id":"call","team_id":"team","metadata":{}}}"#).unwrap();
    assert_eq!(data.http_method, "POST");
    assert_eq!(data.route, "/chat");
    assert_eq!(data.url_path.as_deref(), Some("/chat"));
    let identity = data.identity.unwrap();
    assert_eq!(identity.call_id.as_deref(), Some("call"));
    assert_eq!(identity.team_id.as_deref(), Some("team"));
}
