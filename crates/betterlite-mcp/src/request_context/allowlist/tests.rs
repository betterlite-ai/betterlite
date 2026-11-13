#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::identity_rows::Payload;
use http::HeaderMap;
use http::HeaderName;
use http::header::HeaderValue;
use rstest::rstest;

use super::{
    CLIENT_ID_JWT_FIELD_SETTING, ForbiddenError, IdentitySource, MCP_ALLOWED_CLIENTS_SETTING,
    MCP_CLIENT_ID_HEADER_SETTING, McpClientAllowlist, McpClientAllowlistSettings,
    McpClientForbiddenBody, McpClientIdentity, McpClientRejection, check_mcp_client_allowed,
    load_mcp_client_allowlist, resolve_mcp_client_identity,
};

fn claims(raw: &str) -> Payload {
    serde_json::from_str(raw).expect("claims should parse")
}

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(
            HeaderName::try_from(*name).expect("header name should be valid"),
            HeaderValue::from_str(value).expect("header value should be valid"),
        );
    }
    map
}

fn one_client_allowlist() -> McpClientAllowlist {
    McpClientAllowlist {
        aliases_by_value: BTreeMap::from([("client-1".to_owned(), "app one".to_owned())]),
        jwt_field: Some("azp".to_owned()),
        header: Some("x-client-id".to_owned()),
    }
}

fn rejection_for(
    allowlist: &McpClientAllowlist,
    claims_raw: &str,
    pairs: &[(&str, &str)],
) -> String {
    check_mcp_client_allowed(Some(allowlist), Some(&claims(claims_raw)), &headers(pairs))
        .expect_err("the call should be rejected")
        .details
}

#[test]
fn the_allowlist_is_off_when_the_setting_is_absent() {
    assert_eq!(
        load_mcp_client_allowlist(&McpClientAllowlistSettings::default()),
        None
    );
}

#[test]
fn a_null_client_list_leaves_the_allowlist_off() {
    let settings: McpClientAllowlistSettings =
        serde_json::from_str(r#"{"mcp_allowed_clients": null}"#).expect("settings should parse");
    assert_eq!(load_mcp_client_allowlist(&settings), None);
}

#[test]
fn an_empty_client_list_admits_nobody() {
    let settings: McpClientAllowlistSettings =
        serde_json::from_str(r#"{"mcp_allowed_clients": []}"#).expect("settings should parse");
    let allowlist = load_mcp_client_allowlist(&settings).expect("an empty list is still enforced");
    assert!(allowlist.aliases_by_value.is_empty());
    assert_eq!(allowlist.jwt_field, None);
    assert_eq!(allowlist.header, None);
}

#[rstest]
#[case::blank_alias(r#"{"alias":"","value":"client-1"}"#)]
#[case::blank_value(r#"{"alias":"app two","value":""}"#)]
fn a_blank_alias_or_value_admits_nobody_not_even_the_other_entries(#[case] blank_entry: &str) {
    let settings: McpClientAllowlistSettings = serde_json::from_str(&format!(
        r#"{{"mcp_allowed_clients":[{{"alias":"app one","value":"client-2"}},{blank_entry}],"mcp_client_id_header":"X-Client-ID"}}"#
    ))
    .expect("settings should parse");
    let allowlist =
        load_mcp_client_allowlist(&settings).expect("a malformed list is still enforced");
    assert!(allowlist.aliases_by_value.is_empty());
    assert_eq!(allowlist.header.as_deref(), Some("x-client-id"));
    let rejection = check_mcp_client_allowed(
        Some(&allowlist),
        None,
        &headers(&[("x-client-id", "client-2")]),
    )
    .expect_err("every client is rejected");
    assert!(rejection.details.contains("is not listed"));
}

#[test]
fn each_pair_maps_its_value_to_the_alias_the_admin_gave() {
    let settings: McpClientAllowlistSettings = serde_json::from_str(
        r#"{"mcp_allowed_clients":[{"alias":"app one","value":"client-1"},{"alias":"app two","value":"client-2"}]}"#,
    )
    .expect("settings should parse");
    let allowlist = load_mcp_client_allowlist(&settings).expect("the list should be enforced");
    assert_eq!(
        allowlist
            .aliases_by_value
            .get("client-1")
            .map(String::as_str),
        Some("app one")
    );
    assert_eq!(
        allowlist
            .aliases_by_value
            .get("client-2")
            .map(String::as_str),
        Some("app two")
    );
}

#[test]
fn a_later_pair_for_the_same_value_wins_as_the_python_dict_does() {
    let settings: McpClientAllowlistSettings = serde_json::from_str(
        r#"{"mcp_allowed_clients":[{"alias":"first","value":"client-1"},{"alias":"second","value":"client-1"}]}"#,
    )
    .expect("settings should parse");
    let allowlist = load_mcp_client_allowlist(&settings).expect("the list should be enforced");
    assert_eq!(
        allowlist
            .aliases_by_value
            .get("client-1")
            .map(String::as_str),
        Some("second")
    );
}

#[rstest]
#[case::upper("X-Client-ID", Some("x-client-id"))]
#[case::lower("x-client-id", Some("x-client-id"))]
#[case::empty("", None)]
fn the_header_setting_is_lowercased_and_blank_means_unset(
    #[case] configured: &str,
    #[case] expected: Option<&str>,
) {
    let settings = McpClientAllowlistSettings {
        mcp_allowed_clients: Some(Vec::new()),
        mcp_client_id_header: Some(configured.to_owned()),
        litellm_jwtauth: None,
    };
    let allowlist = load_mcp_client_allowlist(&settings).expect("the list should be enforced");
    assert_eq!(allowlist.header.as_deref(), expected);
}

#[test]
fn the_jwt_field_comes_from_the_jwt_auth_settings() {
    let settings: McpClientAllowlistSettings = serde_json::from_str(
        r#"{"mcp_allowed_clients":[],"litellm_jwtauth":{"mcp_client_id_jwt_field":"realm.client_id"}}"#,
    )
    .expect("settings should parse");
    let allowlist = load_mcp_client_allowlist(&settings).expect("the list should be enforced");
    assert_eq!(allowlist.jwt_field.as_deref(), Some("realm.client_id"));
}

#[rstest]
#[case::empty(r#"{"mcp_allowed_clients":[],"litellm_jwtauth":{"mcp_client_id_jwt_field":""}}"#)]
#[case::absent(r#"{"mcp_allowed_clients":[],"litellm_jwtauth":{}}"#)]
#[case::no_jwt_auth(r#"{"mcp_allowed_clients":[]}"#)]
fn a_blank_or_missing_jwt_field_means_unset(#[case] raw: &str) {
    let settings: McpClientAllowlistSettings =
        serde_json::from_str(raw).expect("settings should parse");
    let allowlist = load_mcp_client_allowlist(&settings).expect("the list should be enforced");
    assert_eq!(allowlist.jwt_field, None);
}

#[test]
fn a_jwt_caller_is_identified_by_the_configured_claim() {
    let identity = resolve_mcp_client_identity(
        &one_client_allowlist(),
        Some(&claims(r#"{"azp":"client-1"}"#)),
        &headers(&[]),
    )
    .expect("the caller should be identified");
    assert_eq!(
        identity,
        McpClientIdentity {
            client_id: "client-1".to_owned(),
            source: IdentitySource::Jwt,
            source_name: "azp".to_owned(),
        }
    );
    assert_eq!(identity.description(), "'client-1' (from JWT claim 'azp')");
}

#[test]
fn a_jwt_caller_never_falls_back_to_a_header() {
    let rejection = rejection_for(
        &one_client_allowlist(),
        r#"{"azp":"client-2"}"#,
        &[("x-client-id", "client-1")],
    );
    assert_eq!(
        rejection,
        "MCP client 'client-2' (from JWT claim 'azp') is not listed in this gateway's mcp_allowed_clients."
    );
}

#[rstest]
#[case::absent("{}")]
#[case::number(r#"{"azp":7}"#)]
#[case::boolean(r#"{"azp":true}"#)]
#[case::object(r#"{"azp":{"name":"client-1"}}"#)]
#[case::list(r#"{"azp":["client-1"]}"#)]
#[case::null(r#"{"azp":null}"#)]
fn a_claim_that_is_not_a_nonempty_text_is_rejected(#[case] claims_raw: &str) {
    let rejection = rejection_for(
        &one_client_allowlist(),
        claims_raw,
        &[("x-client-id", "client-1")],
    );
    assert_eq!(
        rejection,
        "The JWT presented has no 'azp' claim naming the client application. This gateway only admits client applications listed in mcp_allowed_clients."
    );
}

#[test]
fn an_empty_claim_is_rejected() {
    let rejection = rejection_for(&one_client_allowlist(), r#"{"azp":""}"#, &[]);
    assert_eq!(
        rejection,
        "The JWT presented has no 'azp' claim naming the client application. This gateway only admits client applications listed in mcp_allowed_clients."
    );
}

#[test]
fn a_jwt_caller_with_no_configured_field_uses_the_header_instead() {
    let allowlist = McpClientAllowlist {
        aliases_by_value: BTreeMap::from([("client-1".to_owned(), "app one".to_owned())]),
        jwt_field: None,
        header: Some("x-client-id".to_owned()),
    };
    let identity = resolve_mcp_client_identity(
        &allowlist,
        Some(&claims(r#"{"azp":"client-2"}"#)),
        &headers(&[("x-client-id", "client-1")]),
    )
    .expect("the header should identify the caller");
    assert_eq!(identity.client_id, "client-1");
    assert_eq!(identity.source, IdentitySource::Header);
    assert_eq!(
        identity.description(),
        "'client-1' (from header 'x-client-id')"
    );
}

#[test]
fn a_caller_with_no_configured_identity_source_is_rejected() {
    let allowlist = McpClientAllowlist {
        aliases_by_value: BTreeMap::from([("client-1".to_owned(), "app one".to_owned())]),
        jwt_field: None,
        header: None,
    };
    let rejection = rejection_for(&allowlist, r#"{"azp":"client-1"}"#, &[]);
    assert_eq!(
        rejection,
        "No client identity source is configured for this request; set litellm_jwtauth.mcp_client_id_jwt_field for JWT callers or mcp_client_id_header in general_settings. This gateway only admits client applications listed in mcp_allowed_clients."
    );
    assert_eq!(CLIENT_ID_JWT_FIELD_SETTING, "mcp_client_id_jwt_field");
    assert_eq!(MCP_CLIENT_ID_HEADER_SETTING, "mcp_client_id_header");
    assert_eq!(MCP_ALLOWED_CLIENTS_SETTING, "mcp_allowed_clients");
}

#[test]
fn a_header_caller_is_identified_by_its_header() {
    let identity = resolve_mcp_client_identity(
        &one_client_allowlist(),
        None,
        &headers(&[("x-client-id", "client-1")]),
    )
    .expect("the header should identify the caller");
    assert_eq!(identity.client_id, "client-1");
    assert_eq!(identity.source, IdentitySource::Header);
    assert_eq!(identity.source_name, "x-client-id");
}

#[test]
fn the_header_lookup_ignores_the_request_header_case() {
    assert_eq!(
        check_mcp_client_allowed(
            Some(&one_client_allowlist()),
            None,
            &headers(&[("X-Client-Id", "client-1")]),
        ),
        Ok(())
    );
}

#[rstest]
#[case::missing(vec![])]
#[case::empty(vec![("x-client-id", "")])]
fn a_non_jwt_caller_without_a_header_value_is_rejected(#[case] pairs: Vec<(&str, &str)>) {
    let allowlist = McpClientAllowlist {
        aliases_by_value: BTreeMap::from([("client-1".to_owned(), "app one".to_owned())]),
        jwt_field: None,
        header: Some("x-client-id".to_owned()),
    };
    let rejection = check_mcp_client_allowed(Some(&allowlist), None, &headers(&pairs))
        .expect_err("the call should be rejected")
        .details;
    assert_eq!(
        rejection,
        "The request has no 'x-client-id' header naming the client application. This gateway only admits client applications listed in mcp_allowed_clients."
    );
}

#[rstest]
#[case::nested("realm.client_id", r#"{"realm":{"client_id":"client-1"}}"#)]
#[case::escaped_dot(r"kubernetes\.io.client", r#"{"kubernetes.io":{"client":"client-1"}}"#)]
#[case::metadata_prefix("metadata.azp", r#"{"azp":"client-1"}"#)]
#[case::empty_segment("realm..client_id", r#"{"realm":{"":{"client_id":"client-1"}}}"#)]
fn a_claim_path_is_navigated_like_get_nested_value(#[case] field: &str, #[case] claims_raw: &str) {
    let allowlist = McpClientAllowlist {
        aliases_by_value: BTreeMap::from([("client-1".to_owned(), "app one".to_owned())]),
        jwt_field: Some(field.to_owned()),
        header: None,
    };
    let identity =
        resolve_mcp_client_identity(&allowlist, Some(&claims(claims_raw)), &headers(&[]))
            .expect("the claim should resolve");
    assert_eq!(identity.client_id, "client-1");
    assert_eq!(identity.source_name, field);
}

#[rstest]
#[case::metadata_object("metadata.azp", r#"{"metadata":{"azp":"client-1"}}"#)]
#[case::list_in_the_path("groups.0", r#"{"groups":["client-1"]}"#)]
#[case::empty_path("", r#"{"azp":"client-1"}"#)]
#[case::missing_key("azp", r#"{"other":"client-1"}"#)]
fn a_claim_path_that_does_not_resolve_rejects_the_jwt(
    #[case] field: &str,
    #[case] claims_raw: &str,
) {
    let allowlist = McpClientAllowlist {
        aliases_by_value: BTreeMap::from([("client-1".to_owned(), "app one".to_owned())]),
        jwt_field: Some(field.to_owned()),
        header: None,
    };
    let rejection =
        resolve_mcp_client_identity(&allowlist, Some(&claims(claims_raw)), &headers(&[]))
            .expect_err("the claim should not resolve")
            .details;
    assert_eq!(
        rejection,
        format!(
            "The JWT presented has no '{field}' claim naming the client application. This gateway only admits client applications listed in mcp_allowed_clients."
        )
    );
}

#[test]
fn an_unlisted_header_client_is_rejected_with_its_source() {
    let rejection = check_mcp_client_allowed(
        Some(&one_client_allowlist()),
        None,
        &headers(&[("x-client-id", "client-2")]),
    )
    .expect_err("the call should be rejected")
    .details;
    assert_eq!(
        rejection,
        "MCP client 'client-2' (from header 'x-client-id') is not listed in this gateway's mcp_allowed_clients."
    );
}

#[test]
fn the_jwt_claim_value_is_compared_case_sensitively() {
    let rejection = rejection_for(&one_client_allowlist(), r#"{"azp":"CLIENT-1"}"#, &[]);
    assert_eq!(
        rejection,
        "MCP client 'CLIENT-1' (from JWT claim 'azp') is not listed in this gateway's mcp_allowed_clients."
    );
}

#[test]
fn the_header_value_is_compared_case_sensitively() {
    let rejection = check_mcp_client_allowed(
        Some(&one_client_allowlist()),
        None,
        &headers(&[("x-client-id", "CLIENT-1")]),
    )
    .expect_err("the call should be rejected")
    .details;
    assert_eq!(
        rejection,
        "MCP client 'CLIENT-1' (from header 'x-client-id') is not listed in this gateway's mcp_allowed_clients."
    );
}

#[test]
fn a_listed_client_passes() {
    assert_eq!(
        check_mcp_client_allowed(
            Some(&one_client_allowlist()),
            Some(&claims(r#"{"azp":"client-1"}"#)),
            &headers(&[]),
        ),
        Ok(())
    );
}

#[test]
fn no_allowlist_admits_every_client() {
    assert_eq!(check_mcp_client_allowed(None, None, &headers(&[])), Ok(()));
}

#[test]
fn a_latin_1_header_value_matches_its_configured_text() {
    let mut map = HeaderMap::new();
    map.insert(
        HeaderName::from_static("x-client-id"),
        HeaderValue::from_bytes(b"\xe9").expect("latin-1 bytes are a valid header value"),
    );
    let allowlist = McpClientAllowlist {
        aliases_by_value: BTreeMap::from([("\u{e9}".to_owned(), "app one".to_owned())]),
        jwt_field: None,
        header: Some("x-client-id".to_owned()),
    };
    assert_eq!(
        check_mcp_client_allowed(Some(&allowlist), None, &map),
        Ok(())
    );
}

#[test]
fn a_rejection_renders_the_python_forbidden_body() {
    let rejection = McpClientRejection {
        details: "not allowed".to_owned(),
    };
    assert_eq!(
        rejection.response_body(),
        McpClientForbiddenBody {
            error: ForbiddenError::Forbidden,
            details: "not allowed".to_owned(),
        }
    );
    assert_eq!(
        serde_json::to_string(&rejection.response_body()).expect("the body should serialize"),
        r#"{"error":"Forbidden","details":"not allowed"}"#
    );
}
