use betterlite_error::proxy_error_types::ProxyErrorTypes;
use rstest::rstest;

use crate::oauth_client::document::OAuthUserInfo;
use crate::oauth_client::google::{
    GOOGLE_DISCOVERY_URL, GOOGLE_PROVIDER, GOOGLE_SCOPES, GoogleIdentityError, google_open_id,
};

fn userinfo(text: &str) -> OAuthUserInfo {
    serde_json::from_str(text).unwrap()
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
    let identity = google_open_id(&userinfo(
        r#"{
            "sub": "10769150350006150715113082367",
            "email": "ann@example.test",
            "email_verified": true,
            "given_name": "Ann",
            "family_name": "Lee",
            "name": "Ann Lee",
            "picture": "https://img.example.test/ann.png",
            "locale": "en"
        }"#,
    ))
    .unwrap();
    assert_eq!(
        identity.id.as_deref(),
        Some("10769150350006150715113082367")
    );
    assert_eq!(identity.email.as_deref(), Some("ann@example.test"));
    assert_eq!(identity.first_name.as_deref(), Some("Ann"));
    assert_eq!(identity.last_name.as_deref(), Some("Lee"));
    assert_eq!(identity.display_name.as_deref(), Some("Ann Lee"));
    assert_eq!(
        identity.picture.as_deref(),
        Some("https://img.example.test/ann.png")
    );
    assert_eq!(identity.provider.as_deref(), Some("google"));
    assert!(identity.team_ids.is_empty());
    assert!(identity.user_role.is_none());
    assert!(identity.extra_fields.is_none());
}

#[test]
fn missing_profile_fields_stay_absent() {
    let identity = google_open_id(&userinfo(
        r#"{"sub": "1", "email": "ann@example.test", "email_verified": true}"#,
    ))
    .unwrap();
    assert_eq!(identity.id.as_deref(), Some("1"));
    assert_eq!(identity.email.as_deref(), Some("ann@example.test"));
    assert!(identity.first_name.is_none());
    assert!(identity.last_name.is_none());
    assert!(identity.display_name.is_none());
    assert!(identity.picture.is_none());
    assert_eq!(identity.provider.as_deref(), Some("google"));
    assert!(identity.team_ids.is_empty());
    assert!(identity.user_role.is_none());
    assert!(identity.extra_fields.is_none());
}

#[test]
fn an_unverified_google_user_is_refused() {
    let error = google_open_id(&userinfo(
        r#"{"email": "ann@example.test", "email_verified": false}"#,
    ))
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
#[case::null(r#"{"email": "ann@example.test", "email_verified": null}"#)]
#[case::absent(r#"{"email": "ann@example.test"}"#)]
fn a_verified_flag_that_is_not_true_is_refused(#[case] text: &'static str) {
    let error = google_open_id(&userinfo(text)).unwrap_err();
    assert!(matches!(&error, GoogleIdentityError::Unverified { .. }));
    assert_eq!(error.response().status, 401);
}

#[test]
fn a_verified_userinfo_without_an_email_has_no_email() {
    let identity = google_open_id(&userinfo(r#"{"email_verified": true}"#)).unwrap();
    assert!(identity.id.is_none());
    assert!(identity.email.is_none());
    assert_eq!(identity.provider.as_deref(), Some("google"));
}

#[test]
fn an_unverified_google_userinfo_does_not_need_typed_profile_fields() {
    let error = google_open_id(&userinfo(r#"{"email": 7, "email_verified": false}"#)).unwrap_err();
    assert!(matches!(&error, GoogleIdentityError::Unverified { .. }));
    assert_eq!(error.to_string(), "User None is not verified with Google");
}

#[rstest]
#[case::email_verified_text(r#"{"email": "a@b.test", "email_verified": "true"}"#, "email_verified")]
#[case::picture_number(
    r#"{"email": "a@b.test", "email_verified": true, "picture": 7}"#,
    "picture"
)]
#[case::name_object(r#"{"email": "a@b.test", "email_verified": true, "name": {}}"#, "name")]
fn a_google_field_of_the_wrong_type_names_the_field(
    #[case] text: &'static str,
    #[case] field: &'static str,
) {
    let error = google_open_id(&userinfo(text)).unwrap_err();
    let response = error.response();
    assert_eq!(response.status, 503);
    assert_eq!(response.param, Some(field));
}
