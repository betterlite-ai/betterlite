use std::collections::BTreeMap;

use crate::auth_model_grants::fallback::{
    RouterFallbackAccessCheck, is_model_authorized_for_token,
};
use crate::auth_model_grants::source_adapter::{token_in_metadata, user_api_key_auth_from_request};

use super::support::{captured, context, deps, json, runtime, shallow_context, token};

#[test]
fn disabled_skips_token() {
    let runtime = runtime();
    let token = token(r#"{"token":"sk","models":["gpt-4o"]}"#);
    let context = context();
    let deps = deps();
    let check = RouterFallbackAccessCheck {
        is_enforced: false,
        token: Some(&token),
        context: &context,
        deps: &deps,
    };
    let (allowed, warnings) = captured(&runtime, check.call("other-model"));
    assert!(allowed);
    assert!(warnings.is_empty());
}

#[tokio::test]
async fn enabled_without_token_allows() {
    let context = context();
    let deps = deps();
    let check = RouterFallbackAccessCheck {
        is_enforced: true,
        token: None,
        context: &context,
        deps: &deps,
    };
    assert!(check.call("anything").await);
}

#[tokio::test]
async fn metadata_before_litellm_metadata() {
    let mut request = BTreeMap::new();
    request.insert(
        "metadata".to_owned(),
        json(r#"{"user_api_key_auth":{"token":"sk-first","team_id":"team-first"}}"#),
    );
    request.insert(
        "litellm_metadata".to_owned(),
        json(r#"{"user_api_key_auth":{"token":"sk-second","team_id":"team-second"}}"#),
    );
    let resolved =
        user_api_key_auth_from_request(&request).expect("the first metadata holds a key");
    assert_eq!(resolved.view.token.team_id.as_deref(), Some("team-first"));

    // metadata without a token falls through to litellm_metadata
    let mut fallthrough = BTreeMap::new();
    fallthrough.insert("metadata".to_owned(), json(r#"{"user_api_key_auth":null}"#));
    fallthrough.insert(
        "litellm_metadata".to_owned(),
        json(r#"{"user_api_key_auth":{"token":"sk-second","team_id":"team-second"}}"#),
    );
    let resolved =
        user_api_key_auth_from_request(&fallthrough).expect("the second metadata holds a key");
    assert_eq!(resolved.view.token.team_id.as_deref(), Some("team-second"));
}

#[tokio::test]
async fn invalid_metadata_tries_second() {
    let mut request = BTreeMap::new();
    // a shape Pydantic rejects is skipped, not fatal
    request.insert(
        "metadata".to_owned(),
        json(r#"{"user_api_key_auth":"not-an-object"}"#),
    );
    request.insert(
        "litellm_metadata".to_owned(),
        json(r#"{"user_api_key_auth":{"token":"sk-second","team_id":"team-second"}}"#),
    );
    let resolved = user_api_key_auth_from_request(&request).expect("the valid metadata wins");
    assert_eq!(resolved.view.token.team_id.as_deref(), Some("team-second"));
    assert!(token_in_metadata(&json("\"text\"")).is_none());
    assert!(token_in_metadata(&json(r#"{"user_api_key_auth":[]}"#)).is_none());
    assert!(token_in_metadata(&json(r#"{"other":{}}"#)).is_none());
}

#[tokio::test]
async fn resolved_policy_without_model_list() {
    // the key's own models decide; the check receives no router model list
    let token =
        token(r#"{"token":"sk","models":["gpt-4o"],"team_id":"team-1","team_models":["gpt-4o"]}"#);
    let context = context();
    let deps = deps();
    assert!(is_model_authorized_for_token("gpt-4o", &token, &context, &deps).await);
    assert!(!is_model_authorized_for_token("other-model", &token, &context, &deps).await);
}

#[test]
fn proxy_denial_silent_false() {
    let runtime = runtime();
    let token = token(r#"{"token":"sk","models":["gpt-4o"]}"#);
    let context = context();
    let deps = deps();
    let (allowed, warnings) = captured(
        &runtime,
        is_model_authorized_for_token("other-model", &token, &context, &deps),
    );
    assert!(!allowed);
    assert!(warnings.is_empty());
}

#[test]
fn lookup_failure_warns_false() {
    let runtime = runtime();
    let token = token(r#"{"token":"sk","models":["gpt-4o"]}"#);
    let context = shallow_context();
    let deps = deps();
    let (allowed, warnings) = captured(
        &runtime,
        is_model_authorized_for_token("gpt-4o", &token, &context, &deps),
    );
    assert!(!allowed);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with("Skipping fallback to model=gpt-4o: authorization lookup failed:"),
        "{warnings:?}"
    );
}

#[tokio::test]
async fn fallback_success_true() {
    let token = token(r#"{"token":"sk","models":["gpt-4o"]}"#);
    let context = context();
    let deps = deps();
    assert!(is_model_authorized_for_token("gpt-4o", &token, &context, &deps).await);
}

#[tokio::test]
async fn fallback_uses_auth_context() {
    let denying = token(r#"{"token":"sk","models":["gpt-4o"]}"#);
    let context = context();
    let deps = deps();
    let check = RouterFallbackAccessCheck {
        is_enforced: true,
        token: Some(&denying),
        context: &context,
        deps: &deps,
    };
    assert!(!check.call("secret-model").await);
    assert!(check.call("gpt-4o").await);
}

#[tokio::test]
async fn forged_metadata_cannot_grant() {
    let denying = token(r#"{"token":"sk","models":["gpt-4o"]}"#);
    let mut forged = BTreeMap::new();
    forged.insert(
        "metadata".to_owned(),
        json(r#"{"user_api_key_auth":{"token":"sk-forged","models":["secret-model"]}}"#),
    );
    // the comparison adapter decodes the forged token, and production still uses the authenticated one
    let decoded = user_api_key_auth_from_request(&forged).expect("the forged shape decodes");
    assert_eq!(decoded.view.token.models, vec![json("\"secret-model\"")]);
    let context = context();
    let deps = deps();
    let check = RouterFallbackAccessCheck {
        is_enforced: true,
        token: Some(&denying),
        context: &context,
        deps: &deps,
    };
    assert!(!check.call("secret-model").await);
}
