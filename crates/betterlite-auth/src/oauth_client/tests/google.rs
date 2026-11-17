use betterlite_error::proxy_error_types::ProxyErrorTypes;
use rstest::rstest;
use serde_json::json;

use crate::oauth_client::document::OAuthUserInfo;
use crate::oauth_client::google::{
    GOOGLE_DISCOVERY_URL, GOOGLE_PROVIDER, GOOGLE_SCOPES, GoogleIdentityError, google_open_id,
};

fn userinfo(value: serde_json::Value) -> OAuthUserInfo {
    serde_json::from_value(value).unwrap()
}

#[test]
fn the_google_login_constants_match_lite_llm() {
    assert_eq!(
        GOOGLE_DISCOVERY_URL,
        "https://accounts.google.com/.well-known/openid-configuration"
    );
    assert_eq!(GOOGLE_SCOPES, ["openid", "email", "profile"]);
    assert_eq!(GOOGLE_PROVIDER, "google");
}

#[test]
fn a_verified_google_userinfo_becomes_an_open_id_identity() {
    let identity = google_open_id(&userinfo(json!({
        "sub": "10769150350006150715113082367",
        "email": "ann@example.test",
        "email_verified": true,
        "given_name": "Ann",
        "family_name": "Lee",
        "name": "Ann Lee",
        "picture": "https://img.example.test/ann.png",
        "locale": "en",
    })))
    .unwrap();
    assert_eq!(
        serde_json::to_value(identity).unwrap(),
        json!({
            "id": "10769150350006150715113082367",
            "email": "ann@example.test",
            "first_name": "Ann",
            "last_name": "Lee",
            "display_name": "Ann Lee",
            "picture": "https://img.example.test/ann.png",
            "provider": "google",
            "team_ids": [],
            "user_role": null,
            "extra_fields": null,
        })
    );
}

#[test]
fn missing_profile_fields_stay_absent() {
    let identity = google_open_id(&userinfo(json!({
        "sub": "1",
        "email": "ann@example.test",
        "email_verified": true,
    })))
    .unwrap();
    assert_eq!(
        serde_json::to_value(identity).unwrap(),
        json!({
            "id": "1",
            "email": "ann@example.test",
            "first_name": null,
            "last_name": null,
            "display_name": null,
            "picture": null,
            "provider": "google",
            "team_ids": [],
            "user_role": null,
            "extra_fields": null,
        })
    );
}

#[test]
fn an_unverified_google_user_is_refused() {
    let error = google_open_id(&userinfo(json!({
        "email": "ann@example.test",
        "email_verified": false,
    })))
    .unwrap_err();
    assert!(matches!(&error, GoogleIdentityError::Unverified { .. }));
    let response = error.response();
    assert_eq!(response.status, 401);
    assert_eq!(response.error_type, ProxyErrorTypes::AuthError);
    assert_eq!(
        error.to_string(),
        "User ann@example.test is not verified with Google"
    );
}

#[rstest]
#[case::null(json!({"email": "ann@example.test", "email_verified": null}))]
#[case::absent(json!({"email": "ann@example.test"}))]
#[case::verified_without_email(json!({"email_verified": true}))]
fn a_verified_flag_that_is_not_true_is_refused(#[case] value: serde_json::Value) {
    let error = google_open_id(&userinfo(value)).unwrap_err();
    assert!(matches!(&error, GoogleIdentityError::Unverified { .. }));
    assert_eq!(error.response().status, 401);
}

#[test]
fn an_unverified_google_userinfo_does_not_need_typed_profile_fields() {
    let error = google_open_id(&userinfo(json!({
        "email": 7,
        "email_verified": false,
    })))
    .unwrap_err();
    assert!(matches!(&error, GoogleIdentityError::Unverified { .. }));
    assert_eq!(error.to_string(), "User None is not verified with Google");
}

#[rstest]
#[case::email_verified_text(json!({"email": "a@b.test", "email_verified": "true"}), "email_verified")]
#[case::picture_number(json!({"email": "a@b.test", "email_verified": true, "picture": 7}), "picture")]
#[case::name_object(json!({"email": "a@b.test", "email_verified": true, "name": {}}), "name")]
fn a_google_field_of_the_wrong_type_names_the_field(
    #[case] value: serde_json::Value,
    #[case] field: &'static str,
) {
    let error = google_open_id(&userinfo(value)).unwrap_err();
    let response = error.response();
    assert_eq!(response.status, 503);
    assert_eq!(response.param, Some(field));
}
