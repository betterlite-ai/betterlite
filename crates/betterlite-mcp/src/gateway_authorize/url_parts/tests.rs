#![cfg(test)]

use super::*;

#[test]
fn cookie_name_is_the_prefix_plus_the_handle() {
    assert_eq!(flow_cookie_name("h-1"), "mcp_connect_flow_h-1");
}

#[test]
fn cookie_path_keeps_the_base_path_and_marks_https_secure() {
    assert_eq!(
        cookie_path_and_secure("https://proxy.test:8443").unwrap(),
        ("/".to_owned(), true)
    );
    assert_eq!(
        cookie_path_and_secure("http://proxy.test/tenant").unwrap(),
        ("/tenant".to_owned(), false)
    );
}

#[test]
fn an_unreadable_base_url_becomes_a_typed_server_error() {
    let error = OauthError::from(cookie_path_and_secure("http://[::1").unwrap_err());
    assert_eq!(error.status, 500);
    assert_eq!(error.error, OauthErrorCode::ServerError);
    assert_eq!(error.description, UNREADABLE_URL_DESCRIPTION);
}

#[test]
fn existing_query_is_reencoded_and_new_pairs_appended() {
    let url = append_query_params(
        "http://h/cb?x=1&empty=&plus=a+b&pct=a%20b#frag",
        &[
            ("code".to_owned(), "c 1".to_owned()),
            ("state".to_owned(), String::new()),
        ],
    )
    .unwrap();
    assert_eq!(
        url,
        "http://h/cb?x=1&empty=&plus=a+b&pct=a+b&code=c+1&state=#frag"
    );
}

#[test]
fn a_piece_without_equals_keeps_a_blank_value() {
    assert_eq!(
        append_query_params("http://h/cb?bare", &[]).unwrap(),
        "http://h/cb?bare="
    );
}

#[test]
fn quote_plus_leaves_only_the_python_safe_characters() {
    assert_eq!(
        urlencode(&[("q".to_owned(), "a b*~_.-".to_owned())]),
        "q=a+b%2A~_.-"
    );
    assert_eq!(urlencode(&[]), "");
}

#[test]
fn relative_request_url_drops_an_absent_or_empty_query() {
    assert_eq!(relative_request_url("/authorize", None), "/authorize");
    assert_eq!(relative_request_url("/authorize", Some("")), "/authorize");
    assert_eq!(
        relative_request_url("/authorize", Some("a=1&b=2")),
        "/authorize?a=1&b=2"
    );
}

#[test]
fn origin_only_keeps_the_scheme_and_host() {
    assert_eq!(
        origin_only("http://127.0.0.1:8080/cb?x=1").unwrap(),
        "http://127.0.0.1:8080"
    );
    assert_eq!(origin_only("cursor://app/cb").unwrap(), "cursor://app");
    assert_eq!(origin_only("/relative").unwrap(), "");
}
