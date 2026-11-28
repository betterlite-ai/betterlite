#![cfg(test)]

use crate::byok_consent::get::{GetQuery, authorize_get};
use crate::byok_consent::test_support::{page_query as query, registry, server};

fn with_server(json: &str) -> String {
    let query = GetQuery {
        server_id: Some("srv-1".to_owned()),
        ..query()
    };
    authorize_get(&query, Some(&registry(vec![server(json)]))).unwrap()
}

#[test]
fn a_script_in_the_client_id_is_escaped() {
    let page_query = GetQuery {
        client_id: Some("<script>alert(1)</script>".to_owned()),
        ..query()
    };
    let html = authorize_get(&page_query, None).unwrap();
    assert!(html.contains("name=\"client_id\" value=\"&lt;script&gt;alert(1)&lt;/script&gt;\""));
    assert!(!html.contains("<script>alert"));
}

#[test]
fn the_state_escapes_quotes_and_ampersands() {
    let page_query = GetQuery {
        state: Some("\"'&".to_owned()),
        ..query()
    };
    let html = authorize_get(&page_query, None).unwrap();
    assert!(html.contains("name=\"state\" value=\"&quot;&#x27;&amp;\""));
}

#[test]
fn the_code_challenge_is_escaped() {
    let page_query = GetQuery {
        code_challenge: Some("a\"b<c".to_owned()),
        ..query()
    };
    let html = authorize_get(&page_query, None).unwrap();
    assert!(html.contains("name=\"code_challenge\" value=\"a&quot;b&lt;c\""));
}

#[test]
fn a_quote_in_the_challenge_cannot_add_a_form_field() {
    let page_query = GetQuery {
        code_challenge: Some("x\"><input name=\"admin\" value=\"1".to_owned()),
        ..query()
    };
    let html = authorize_get(&page_query, None).unwrap();
    assert!(html.contains("value=\"x&quot;&gt;&lt;input name=&quot;admin&quot; value=&quot;1\""));
    assert!(!html.contains("<input name=\"admin\""));
}

#[test]
fn the_server_name_is_escaped_everywhere_it_appears() {
    let html = with_server(
        r#"{"server_id": "srv-1", "name": "evil", "server_name": "Evil \"><script>alert(1)</script>", "transport": "sse"}"#,
    );
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(!html.contains("<script>alert"));
}

#[test]
fn the_monogram_is_escaped() {
    let html = with_server(
        r#"{"server_id": "srv-1", "name": "evil", "server_name": "<evil>", "transport": "sse"}"#,
    );
    assert!(html.contains("class=\"logo logo-s\">&lt;<"));
}

#[test]
fn an_access_item_is_escaped() {
    let html = with_server(
        r#"{"server_id": "srv-1", "name": "evil", "transport": "sse", "byok_description": ["<img src=x onerror=alert(1)>"]}"#,
    );
    assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    assert!(!html.contains("<img src=x"));
}

#[test]
fn a_javascript_help_url_is_suppressed() {
    let html = with_server(
        r#"{"server_id": "srv-1", "name": "evil", "transport": "sse", "byok_api_key_help_url": "javascript:alert(1)"}"#,
    );
    assert!(!html.contains("class=\"help-link\""));
    assert!(!html.contains("javascript:alert"));
}

#[test]
fn a_data_help_url_is_suppressed() {
    let html = with_server(
        r#"{"server_id": "srv-1", "name": "evil", "transport": "sse", "byok_api_key_help_url": "data:text/html,<script>alert(1)</script>"}"#,
    );
    assert!(!html.contains("class=\"help-link\""));
}

#[test]
fn an_https_help_url_is_kept() {
    let html = with_server(
        r#"{"server_id": "srv-1", "name": "github", "transport": "sse", "byok_api_key_help_url": "https://github.com/settings/tokens"}"#,
    );
    assert!(html.contains("class=\"help-link\" href=\"https://github.com/settings/tokens\""));
}

#[test]
fn a_quote_in_the_help_url_is_escaped() {
    let html = with_server(
        r#"{"server_id": "srv-1", "name": "evil", "transport": "sse", "byok_api_key_help_url": "https://example.com/?a=\"onmouseover=alert(1)"}"#,
    );
    assert!(html.contains("href=\"https://example.com/?a=&quot;onmouseover=alert(1)\""));
}
