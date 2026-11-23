use std::collections::BTreeMap;

use rstest::rstest;

use crate::openapi_tools::request_headers::{OpenApiCallContext, merge_request_headers};

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn context(pairs: &[(&str, &str)]) -> OpenApiCallContext {
    OpenApiCallContext {
        extra_headers: headers(pairs),
        ..OpenApiCallContext::default()
    }
}

#[test]
fn an_empty_merge_stays_empty() {
    assert!(merge_request_headers(&BTreeMap::new(), &OpenApiCallContext::default()).is_empty());
}

#[test]
fn static_headers_are_kept() {
    let static_headers = headers(&[("X-Tenant", "acme"), ("X-Key", "k")]);
    assert_eq!(
        merge_request_headers(&static_headers, &OpenApiCallContext::default()),
        static_headers
    );
}

#[test]
fn extra_headers_are_forwarded_without_a_clash() {
    let static_headers = headers(&[("X-Tenant", "acme")]);
    let merged = merge_request_headers(&static_headers, &context(&[("X-Trace", "t")]));
    assert_eq!(merged, headers(&[("X-Tenant", "acme"), ("X-Trace", "t")]));
}

#[rstest]
#[case::same_case("X-Tenant")]
#[case::other_case("x-tenant")]
#[case::mixed_case("X-TENANT")]
fn a_static_header_beats_an_extra_header(#[case] extra_name: &str) {
    let static_headers = headers(&[("X-Tenant", "acme")]);
    let merged = merge_request_headers(&static_headers, &context(&[(extra_name, "caller")]));
    assert_eq!(merged, headers(&[("X-Tenant", "acme")]));
}

#[test]
fn the_static_name_keeps_its_casing() {
    let static_headers = headers(&[("Authorization", "Bearer static")]);
    let merged = merge_request_headers(
        &static_headers,
        &context(&[("authorization", "Bearer caller")]),
    );
    assert_eq!(merged, headers(&[("Authorization", "Bearer static")]));
}

#[test]
fn a_byok_header_replaces_every_authorization_casing() {
    let static_headers = headers(&[("authorization", "Bearer static"), ("X-Tenant", "acme")]);
    let byok = OpenApiCallContext {
        auth_header: Some("Bearer byok".to_owned()),
        ..OpenApiCallContext::default()
    };
    let merged = merge_request_headers(&static_headers, &byok);
    assert_eq!(
        merged,
        headers(&[("Authorization", "Bearer byok"), ("X-Tenant", "acme")])
    );
}

#[test]
fn an_empty_byok_header_changes_nothing() {
    let static_headers = headers(&[("Authorization", "Bearer static")]);
    let byok = OpenApiCallContext {
        auth_header: Some(String::new()),
        ..OpenApiCallContext::default()
    };
    assert_eq!(
        merge_request_headers(&static_headers, &byok),
        static_headers
    );
}

#[test]
fn a_resolved_authorization_beats_byok_and_static() {
    let static_headers = headers(&[("Authorization", "Bearer static")]);
    let resolved = OpenApiCallContext {
        auth_header: Some("Bearer byok".to_owned()),
        resolved_auth_headers: headers(&[("authorization", "Bearer resolved")]),
        ..OpenApiCallContext::default()
    };
    assert_eq!(
        merge_request_headers(&static_headers, &resolved),
        headers(&[("authorization", "Bearer resolved")])
    );
}

#[test]
fn resolved_custom_headers_replace_the_same_name_in_any_casing() {
    let static_headers = headers(&[("X-Cred", "static"), ("X-Tenant", "acme")]);
    let resolved = OpenApiCallContext {
        resolved_auth_headers: headers(&[("x-cred", "resolved")]),
        ..OpenApiCallContext::default()
    };
    assert_eq!(
        merge_request_headers(&static_headers, &resolved),
        headers(&[("x-cred", "resolved"), ("X-Tenant", "acme")])
    );
}

#[test]
fn the_whole_precedence_chain_applies_at_once() {
    let static_headers = headers(&[("X-Tenant", "static"), ("Authorization", "Bearer static")]);
    let context = OpenApiCallContext {
        auth_header: Some("Bearer byok".to_owned()),
        extra_headers: headers(&[
            ("x-tenant", "extra"),
            ("X-Trace", "t"),
            ("Authorization", "Bearer extra"),
        ]),
        resolved_auth_headers: headers(&[("X-Cred", "resolved")]),
    };
    assert_eq!(
        merge_request_headers(&static_headers, &context),
        headers(&[
            ("Authorization", "Bearer byok"),
            ("X-Cred", "resolved"),
            ("X-Tenant", "static"),
            ("X-Trace", "t"),
        ])
    );
}

#[test]
fn the_static_headers_are_not_changed() {
    let static_headers = headers(&[("X-Tenant", "acme")]);
    let before = static_headers.clone();
    let context = OpenApiCallContext {
        auth_header: Some("Bearer byok".to_owned()),
        ..OpenApiCallContext::default()
    };
    drop(merge_request_headers(&static_headers, &context));
    assert_eq!(static_headers, before);
}
