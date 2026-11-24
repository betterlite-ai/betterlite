#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use http::HeaderMap;
use http::header::{AUTHORIZATION, HeaderName, HeaderValue};
use rstest::rstest;
use serde_json::json;

use super::{
    authorization_header, forwarded_authorization, owner_fingerprint, servers_in_path,
    session_id_header,
};

// sha256 of the credential each fixture hands to the fingerprint, or of the value construct stores
const SK_HASH: &str = "453ba228e6f153141f3557f105b19afe1bb4fb7ac3181c1a9e4f6ec144761b82";
const CUSTOM_HASH: &str = "1481b6fd3ceddb4ae3c27c41a53f483681c3bb937c95fed0fb706b1494707221";
const JWT_HASH: &str = "544dad3ed099ad2e72763ff351333c53297cd67ea1ebf3e092846c532bf74687";
const USER_HASH: &str = "c6c289e49e9c05b2145860387b73bcb18df43fb09a1e4a4a9713c76c88bb541b";
const OAUTH_HASH: &str = "13f7f7e725757b6e7b9350762c0036c30f2cd01e951d2683d16dbc2fbe172684";
const LOWER_OAUTH_HASH: &str = "b449a712bac0f7b6d605d2844c954ad21981146291c045222b7ff01207d872d8";
const IP_HASH: &str = "f5047344122f0dee9974ba6761e61c6b8649e1f3968d13a635ebbf7be53a3a0d";

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    let built: Vec<(HeaderName, HeaderValue)> = pairs
        .iter()
        .map(|(name, value)| {
            (
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_bytes(value.as_bytes()).unwrap(),
            )
        })
        .collect();
    for (name, value) in built {
        map.append(name, value);
    }
    map
}

fn auth(pairs: &[(&str, JsonValue)]) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = pairs
        .iter()
        .map(|(key, value)| {
            let decoded = serde_json::from_value(value.clone()).unwrap();
            ((*key).to_owned(), decoded)
        })
        .collect();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

fn oauth(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::sse("/mcp/sse", None)]
#[case::sse_with_trailing_slash("/mcp/sse/", None)]
#[case::sse_messages("/mcp/sse/messages", None)]
#[case::sse_messages_with_trailing_slash("/mcp/sse/messages/", None)]
#[case::server_then_mcp("/foo/mcp", Some(vec!["foo"]))]
#[case::server_then_mcp_with_route("/foo/mcp/tools", Some(vec!["foo"]))]
#[case::mcp_then_mcp("/mcp/mcp", Some(vec!["mcp"]))]
#[case::sse_as_a_server_name("/mcp/sse/events", Some(vec!["sse/events"]))]
#[case::one_segment("/mcp/one", Some(vec!["one"]))]
#[case::two_segments("/mcp/one/two", Some(vec!["one/two"]))]
#[case::three_segments("/mcp/one/two/tools", Some(vec!["one/two"]))]
#[case::comma_list_with_route("/mcp/one,two/tools", Some(vec!["one", "two"]))]
#[case::comma_list_without_route("/mcp/one,two", Some(vec!["one", "two"]))]
#[case::comma_after_a_route("/mcp/a,b/c/d", Some(vec!["a", "b"]))]
#[case::comma_inside_later_segments(
    "/mcp/one,two/three,four",
    Some(vec!["one", "two/three", "four"])
)]
#[case::slash_inside_a_name("/mcp/custom_solutions/user_123", Some(vec!["custom_solutions/user_123"]))]
#[case::slash_inside_a_name_with_a_route(
    "/mcp/custom_solutions/user_123/chat",
    Some(vec!["custom_solutions/user_123"])
)]
#[case::bare_mcp("/mcp", None)]
#[case::bare_mcp_with_slash("/mcp/", None)]
#[case::only_commas("/mcp/,", Some(vec![]))]
#[case::two_commas("/mcp/,,", Some(vec![]))]
#[case::blanks_are_dropped("/mcp/, a ,,b,", Some(vec!["a", "b"]))]
#[case::query_after_a_name("/mcp/one?x=1", Some(vec!["one"]))]
#[case::query_after_a_list("/mcp/one,two/tools?x=1", Some(vec!["one", "two"]))]
#[case::fragment_after_a_name("/mcp/one#frag", Some(vec!["one"]))]
#[case::wrong_prefix("/other/path", None)]
#[case::double_slash_prefix("//mcp/x", None)]
#[case::names_are_trimmed("/mcp/ a , b /tools", Some(vec!["a", "b"]))]
fn servers_in_path_matches_litellm(#[case] path: &str, #[case] expected: Option<Vec<&str>>) {
    let wanted: Option<Vec<String>> =
        expected.map(|names| names.into_iter().map(str::to_owned).collect());
    assert_eq!(servers_in_path(path), wanted);
}

#[test]
fn the_first_session_id_header_wins() {
    let map = headers(&[("mcp-session-id", "one"), ("mcp-session-id", "two")]);
    assert_eq!(session_id_header(&map), Some("one".to_owned()));
}

#[test]
fn a_missing_session_id_is_absent() {
    assert_eq!(session_id_header(&headers(&[])), None);
}

#[test]
fn a_session_id_header_name_is_case_insensitive() {
    let map = headers(&[("MCP-Session-ID", "abc")]);
    assert_eq!(session_id_header(&map), Some("abc".to_owned()));
}

#[test]
fn a_session_id_may_hold_non_ascii_utf8() {
    let mut map = HeaderMap::new();
    map.append(
        "mcp-session-id",
        HeaderValue::from_bytes("na\u{ef}ve".as_bytes()).unwrap(),
    );
    assert_eq!(session_id_header(&map), Some("na\u{ef}ve".to_owned()));
}

#[test]
fn a_session_id_that_is_not_utf8_reads_as_absent() {
    let mut map = HeaderMap::new();
    map.append("mcp-session-id", HeaderValue::from_bytes(b"\xff").unwrap());
    assert_eq!(session_id_header(&map), None);
}

#[test]
fn a_key_outranks_every_other_credential() {
    let caller = auth(&[
        ("api_key", json!("sk-live-123")),
        ("user_id", json!("user-1")),
    ]);
    let upstream = oauth(&[("Authorization", "Bearer oauth-token")]);
    assert_eq!(
        owner_fingerprint(Some(&caller), Some(&upstream), Some("10.0.0.1")),
        format!("key:{SK_HASH}")
    );
}

#[test]
fn a_custom_key_is_hashed_as_stored() {
    let caller = auth(&[("api_key", json!("custom-key"))]);
    assert_eq!(
        owner_fingerprint(Some(&caller), None, None),
        format!("key:{CUSTOM_HASH}")
    );
}

#[test]
fn a_jwt_key_is_hashed_after_its_prefix() {
    let caller = auth(&[("api_key", json!("a.b.c"))]);
    assert_eq!(
        owner_fingerprint(Some(&caller), None, None),
        format!("key:{JWT_HASH}")
    );
}

#[test]
fn an_empty_key_falls_through_to_the_user() {
    let caller = auth(&[("api_key", json!("")), ("user_id", json!("user-1"))]);
    assert_eq!(
        owner_fingerprint(Some(&caller), None, None),
        format!("user:{USER_HASH}")
    );
}

#[test]
fn a_user_without_a_key_is_hashed_as_the_user() {
    let caller = auth(&[("user_id", json!("user-1"))]);
    assert_eq!(
        owner_fingerprint(Some(&caller), None, None),
        format!("user:{USER_HASH}")
    );
}

#[test]
fn the_upper_authorization_header_comes_first() {
    let upstream = oauth(&[
        ("authorization", "lower-token"),
        ("Authorization", "Bearer oauth-token"),
    ]);
    assert_eq!(
        owner_fingerprint(None, Some(&upstream), None),
        format!("oauth:{OAUTH_HASH}")
    );
}

#[test]
fn the_lower_authorization_header_is_used_when_the_upper_is_absent() {
    let upstream = oauth(&[("authorization", "lower-token")]);
    assert_eq!(
        owner_fingerprint(None, Some(&upstream), None),
        format!("oauth:{LOWER_OAUTH_HASH}")
    );
}

#[test]
fn an_empty_upper_authorization_falls_through_to_the_lower_one() {
    let upstream = oauth(&[("Authorization", ""), ("authorization", "lower-token")]);
    assert_eq!(
        owner_fingerprint(None, Some(&upstream), None),
        format!("oauth:{LOWER_OAUTH_HASH}")
    );
}

#[test]
fn an_empty_lower_authorization_is_no_credential() {
    let upstream = oauth(&[("authorization", "")]);
    assert_eq!(
        owner_fingerprint(None, Some(&upstream), Some("10.0.0.1")),
        format!("ip:{IP_HASH}")
    );
}

#[test]
fn the_client_ip_is_the_last_credential() {
    assert_eq!(
        owner_fingerprint(None, None, Some("10.0.0.1")),
        format!("ip:{IP_HASH}")
    );
}

#[test]
fn empty_credentials_fall_through_to_the_client_ip() {
    let caller = auth(&[("api_key", json!("")), ("user_id", json!(""))]);
    let upstream = oauth(&[("Authorization", "")]);
    assert_eq!(
        owner_fingerprint(Some(&caller), Some(&upstream), Some("10.0.0.1")),
        format!("ip:{IP_HASH}")
    );
}

#[test]
fn no_credentials_at_all_is_anonymous() {
    let caller = auth(&[]);
    assert_eq!(
        owner_fingerprint(Some(&caller), Some(&oauth(&[])), Some("")),
        "anonymous"
    );
}

#[test]
fn the_first_authorization_header_wins() {
    let map = headers(&[("authorization", "one"), ("authorization", "two")]);
    assert_eq!(authorization_header(&map), Some("one".to_owned()));
}

#[test]
fn a_missing_authorization_is_absent() {
    assert_eq!(authorization_header(&headers(&[])), None);
}

#[test]
fn an_authorization_value_reads_latin1_bytes() {
    let mut map = HeaderMap::new();
    map.append(
        AUTHORIZATION,
        HeaderValue::from_bytes(b"B\xe9arer").unwrap(),
    );
    assert_eq!(authorization_header(&map), Some("B\u{e9}arer".to_owned()));
}

#[rstest]
#[case::without_the_key(&[("authorization", "Bearer upstream")], None)]
#[case::with_the_key(
    &[("authorization", "Bearer upstream"), ("x-litellm-api-key", "sk-1")],
    Some("Bearer upstream")
)]
#[case::key_without_authorization(&[("x-litellm-api-key", "sk-1")], None)]
#[case::names_are_case_insensitive(
    &[("X-LITELLM-API-KEY", "sk-1"), ("AUTHORIZATION", "tok")],
    Some("tok")
)]
#[case::first_authorization_wins(
    &[
        ("x-litellm-api-key", "sk-1"),
        ("authorization", "one"),
        ("authorization", "two"),
    ],
    Some("one")
)]
#[case::empty_authorization(
    &[("authorization", ""), ("x-litellm-api-key", "sk-1")],
    Some("")
)]
#[case::no_headers(&[], None)]
fn forwarded_authorization_needs_the_litellm_key(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        forwarded_authorization(&headers(pairs)).as_deref(),
        expected
    );
}
