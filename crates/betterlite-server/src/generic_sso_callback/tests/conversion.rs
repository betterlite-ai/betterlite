use betterlite_config::generic_sso_callback::INCLUDE_TOKEN_CLAIMS_ENV;
use betterlite_config::sso_claims::EXTRA_ATTRIBUTES_ENV;

use super::support::{
    FakeCache, FakeRows, Fixture, Replies, TokenReply, attributes, call, callback_settings, pkce,
    text, token,
};

// the client's tokens carry claims the userinfo does not, so a merge shows up in the identity
fn claim_replies() -> Replies {
    let mut replies = Replies::new();
    replies.token = TokenReply {
        access_token: token(r#"{"region":"eu","department":"sales"}"#),
        id_token: Some(token(r#"{"department":"eng"}"#)),
        refresh_token: Some("refresh-1".to_owned()),
    };
    replies
}

fn extra_attributes() -> Vec<(&'static str, &'static str)> {
    vec![(EXTRA_ATTRIBUTES_ENV, "department,region")]
}

#[tokio::test]
async fn token_claims_merge_into_the_identity_when_the_flag_is_on() {
    let mut fixture = Fixture::start(claim_replies()).await;
    fixture.callback_settings = callback_settings(&[(INCLUDE_TOKEN_CLAIMS_ENV, "TRUE")]);
    fixture.attributes = attributes(&extra_attributes());
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap();
    let extra = result.identity.extra_fields.unwrap();
    assert_eq!(
        extra
            .get("department")
            .and_then(|value| value.json.as_str()),
        Some("eng")
    );
    assert_eq!(
        extra.get("region").and_then(|value| value.json.as_str()),
        Some("eu")
    );
}

#[tokio::test]
async fn token_claims_stay_out_when_the_flag_is_off() {
    let mut fixture = Fixture::start(claim_replies()).await;
    fixture.attributes = attributes(&extra_attributes());
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap();
    let extra = result.identity.extra_fields.unwrap();
    assert!(extra.get("department").unwrap().json.is_null());
    assert!(extra.get("region").unwrap().json.is_null());
}

#[tokio::test]
async fn the_received_response_never_carries_a_bearer_token() {
    let mut fixture = Fixture::start(claim_replies()).await;
    fixture.pkce = pkce(true);
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc&state=state-1"),
        Some("state-1"),
        &FakeRows::absent(),
        &FakeCache::holding_verifier(),
    )
    .await
    .unwrap();
    assert!(result.received_response.get("access_token").is_none());
    assert!(result.received_response.get("id_token").is_none());
    assert!(result.received_response.get("refresh_token").is_none());
    assert_eq!(
        result
            .received_response
            .get("sub")
            .and_then(|value| value.json.as_str()),
        Some("subject-1")
    );
    assert_eq!(
        result
            .received_response
            .get("token_type")
            .and_then(|value| value.json.as_str()),
        Some("Bearer")
    );
}

#[tokio::test]
async fn the_response_id_token_wins_over_the_client_one() {
    let mut replies = Replies::new();
    let response_id_token = token(r#"{"department":"eng"}"#);
    replies
        .userinfo
        .insert("id_token".to_owned(), text(&response_id_token));
    replies.token = TokenReply {
        access_token: "access-opaque".to_owned(),
        id_token: Some(token(r#"{"department":"sales"}"#)),
        refresh_token: None,
    };
    let mut fixture = Fixture::start(replies).await;
    fixture.callback_settings = callback_settings(&[(INCLUDE_TOKEN_CLAIMS_ENV, "true")]);
    fixture.attributes = attributes(&[(EXTRA_ATTRIBUTES_ENV, "department")]);
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap();
    let extra = result.identity.extra_fields.unwrap();
    assert_eq!(
        extra
            .get("department")
            .and_then(|value| value.json.as_str()),
        Some("eng")
    );
}

#[tokio::test]
async fn the_client_id_token_fills_in_when_the_response_has_none() {
    let mut replies = Replies::new();
    replies.token = TokenReply {
        access_token: "access-opaque".to_owned(),
        id_token: Some(token(
            r#"{"department":"sales","iss":"https://issuer.test","exp":2000000000}"#,
        )),
        refresh_token: None,
    };
    let mut fixture = Fixture::start(replies).await;
    fixture.callback_settings = callback_settings(&[(INCLUDE_TOKEN_CLAIMS_ENV, "true")]);
    fixture.attributes = attributes(&[(EXTRA_ATTRIBUTES_ENV, "department")]);
    let result = call(
        &fixture,
        &fixture.callback_url("code=abc"),
        None,
        &FakeRows::absent(),
        &FakeCache::empty(),
    )
    .await
    .unwrap();
    let extra = result.identity.extra_fields.unwrap();
    assert_eq!(
        extra
            .get("department")
            .and_then(|value| value.json.as_str()),
        Some("sales")
    );
    assert_eq!(
        result.sso_assertion.unwrap().issuer.as_deref(),
        Some("https://issuer.test")
    );
}
