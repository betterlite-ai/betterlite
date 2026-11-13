#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use super::{CacheControlCallData, CacheControlPermissions, CacheControlPermissionsError};

fn principal(text: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(text).expect("valid JSON fixture");
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("fixture should construct")
}

fn data(text: &str) -> CacheControlCallData {
    serde_json::from_str(text).expect("valid JSON fixture")
}

async fn run(
    auth: &UserAPIKeyAuth,
    data: &CacheControlCallData,
) -> Result<(), CacheControlPermissionsError> {
    CacheControlPermissions
        .async_pre_call_hook(auth, None, data, "completion")
        .await
}

#[rstest]
#[case::no_cache_key(r#"{"allowed_cache_controls": ["ttl"]}"#, "{}")]
#[case::cache_present_but_not_a_dict(
    r#"{"allowed_cache_controls": ["ttl"]}"#,
    r#"{"cache": "no-cache"}"#
)]
#[case::cache_present_as_array(
    r#"{"allowed_cache_controls": ["ttl"]}"#,
    r#"{"cache": ["no-cache"]}"#
)]
#[case::empty_cache_dict(r#"{"allowed_cache_controls": ["ttl"]}"#, r#"{"cache": {}}"#)]
#[case::no_allowlist_permits_any_key("{}", r#"{"cache": {"no-cache": true}}"#)]
#[case::empty_allowlist_permits_any_key(
    r#"{"allowed_cache_controls": []}"#,
    r#"{"cache": {"no-cache": true}}"#
)]
#[case::nonempty_allowlist_permits_listed_key(
    r#"{"allowed_cache_controls": ["no-cache", "no-store"]}"#,
    r#"{"cache": {"no-cache": true, "no-store": true}}"#
)]
#[tokio::test]
async fn permits_the_request(#[case] auth: &str, #[case] data_text: &str) {
    let result = run(&principal(auth), &data(data_text)).await;
    assert!(
        result.is_ok(),
        "expected the request to be permitted, got {result:?}"
    );
}

#[tokio::test]
async fn rejects_a_key_outside_a_nonempty_allowlist() {
    let auth = principal(r#"{"allowed_cache_controls": ["no-cache"]}"#);
    let data = data(r#"{"cache": {"ttl": 60}}"#);

    let error = run(&auth, &data)
        .await
        .expect_err("unauthorized key must be rejected");

    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(
        error.to_string(),
        "Not allowed to set ttl as a cache control. Contact admin to change permissions."
    );
    match error {
        CacheControlPermissionsError::Forbidden { key } => {
            assert_eq!(key, "ttl");
        }
    }
}

#[tokio::test]
async fn rejects_a_key_when_allowlist_contains_nonstring_entries() {
    let auth = principal(r#"{"allowed_cache_controls": ["no-cache", 123]}"#);
    let data = data(r#"{"cache": {"ttl": 60}}"#);

    let error = run(&auth, &data)
        .await
        .expect_err("unauthorized key must be rejected despite mixed allowlist entries");

    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert!(error.to_string().contains("ttl"));
}

#[tokio::test]
async fn rejects_a_key_when_allowlist_has_only_nonstring_entries() {
    let auth = principal(r#"{"allowed_cache_controls": [123]}"#);
    let data = data(r#"{"cache": {"ttl": 60}}"#);

    let error = run(&auth, &data)
        .await
        .expect_err("nonempty allowlist must reject an unlisted key");

    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert!(error.to_string().contains("ttl"));
}
