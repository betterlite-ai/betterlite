#![cfg(test)]

mod caching;
mod support;

use std::sync::Arc;

use betterlite_mcp::credential_contracts::auth_config::oauth::{
    DEFAULT_SUBJECT_TOKEN_TYPE, TokenExchangeProfile,
};
use betterlite_mcp::credential_contracts::errors::{CredError, Unauthorized};
use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointAuthMethod;
use rstest::rstest;
use secrecy::SecretString;

use super::super::v2_store::CredentialValue;
use super::body::parse_expires_in;
use super::cache_key;
use super::{JWT_BEARER_GRANT, ON_BEHALF_OF, TOKEN_EXCHANGE_GRANT};
use support::{
    RecordingPost, Scripted, at, base_config, body, clock, config, exchanger, pairs, server,
};

#[tokio::test]
async fn rfc8693_exchange_sends_the_standard_form_and_returns_the_token() {
    let post = RecordingPost::new(vec![Scripted::Body(body(
        r#"{"access_token":"exchanged","expires_in":3600,"token_type":"Bearer"}"#,
    ))]);
    let exchanger = exchanger(post, clock(1000));
    let token = exchanger
        .exchange("caller-jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(token.access_token, "exchanged");
    assert_eq!(token.expires_at, Some(at(4600)));
    let calls = exchanger.post.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url, "https://idp.example.com/token");
    assert_eq!(
        calls[0].form,
        pairs(&[
            ("audience", "https://up.example.com"),
            ("client_id", "cid"),
            ("client_secret", "csec"),
            ("grant_type", TOKEN_EXCHANGE_GRANT),
            ("scope", "s1 s2"),
            ("subject_token", "caller-jwt"),
            ("subject_token_type", DEFAULT_SUBJECT_TOKEN_TYPE),
        ])
    );
    assert!(calls[0].headers.is_empty());
}

#[tokio::test]
async fn rfc8693_form_omits_an_absent_audience_and_empty_scopes() {
    let post = RecordingPost::new(vec![Scripted::Body(body(r#"{"access_token":"x"}"#))]);
    let exchanger = exchanger(post, clock(1000));
    exchanger
        .exchange("caller-jwt", &server(), &base_config(), "")
        .await
        .unwrap();
    let calls = exchanger.post.calls();
    assert_eq!(
        calls[0].form,
        pairs(&[
            ("client_id", "cid"),
            ("client_secret", "csec"),
            ("grant_type", TOKEN_EXCHANGE_GRANT),
            ("subject_token", "caller-jwt"),
            ("subject_token_type", DEFAULT_SUBJECT_TOKEN_TYPE),
        ])
    );
}

#[tokio::test]
async fn rfc8693_form_omits_an_empty_audience() {
    let post = RecordingPost::new(vec![Scripted::Body(body(r#"{"access_token":"x"}"#))]);
    let exchanger = exchanger(post, clock(1000));
    let mut config = base_config();
    config.audience = Some(String::new());
    exchanger
        .exchange("caller-jwt", &server(), &config, "")
        .await
        .unwrap();
    assert!(!exchanger.post.calls()[0].form.contains_key("audience"));
}

#[tokio::test]
async fn entra_obo_exchange_sends_the_jwt_bearer_form() {
    let post = RecordingPost::new(vec![Scripted::Body(body(
        r#"{"access_token":"minted","expires_in":3600}"#,
    ))]);
    let exchanger = exchanger(post, clock(1000));
    let mut config = base_config();
    config.profile = TokenExchangeProfile::EntraObo;
    config.audience = Some("ignored-in-entra-obo".to_owned());
    config.subject_token_type = "urn:ietf:params:oauth:token-type:jwt".to_owned();
    config.scopes = vec!["api://target-api/.default".to_owned()];
    let token = exchanger
        .exchange("caller-entra-jwt", &server(), &config, "")
        .await
        .unwrap();
    assert_eq!(token.access_token, "minted");
    let calls = exchanger.post.calls();
    assert_eq!(
        calls[0].form,
        pairs(&[
            ("assertion", "caller-entra-jwt"),
            ("client_id", "cid"),
            ("client_secret", "csec"),
            ("grant_type", JWT_BEARER_GRANT),
            ("requested_token_use", ON_BEHALF_OF),
            ("scope", "api://target-api/.default"),
        ])
    );
}

#[tokio::test]
async fn client_secret_basic_sends_the_authorization_header_and_no_form_credentials() {
    let post = RecordingPost::new(vec![Scripted::Body(body(r#"{"access_token":"x"}"#))]);
    let exchanger = exchanger(post, clock(1000));
    let mut config = base_config();
    config.token_endpoint_auth_method = Some(TokenEndpointAuthMethod::ClientSecretBasic);
    exchanger
        .exchange("caller-jwt", &server(), &config, "")
        .await
        .unwrap();
    let calls = exchanger.post.calls();
    assert!(!calls[0].form.contains_key("client_id"));
    assert!(!calls[0].form.contains_key("client_secret"));
    assert_eq!(
        calls[0].headers,
        pairs(&[("Authorization", "Basic Y2lkOmNzZWM=")])
    );
}

#[tokio::test]
async fn client_secret_post_sends_the_credentials_in_the_form() {
    let post = RecordingPost::new(vec![Scripted::Body(body(r#"{"access_token":"x"}"#))]);
    let exchanger = exchanger(post, clock(1000));
    let mut config = base_config();
    config.token_endpoint_auth_method = Some(TokenEndpointAuthMethod::ClientSecretPost);
    exchanger
        .exchange("caller-jwt", &server(), &config, "")
        .await
        .unwrap();
    let calls = exchanger.post.calls();
    assert_eq!(calls[0].form.get("client_id"), Some(&"cid".to_owned()));
    assert_eq!(calls[0].form.get("client_secret"), Some(&"csec".to_owned()));
    assert!(calls[0].headers.is_empty());
}

#[tokio::test]
async fn missing_endpoint_is_precondition_required_without_a_post() {
    let post = RecordingPost::new(vec![]);
    let exchanger = exchanger(post, clock(1000));
    let mut config = base_config();
    config.token_exchange_endpoint = None;
    let error = exchanger
        .exchange("caller-jwt", &server(), &config, "")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::PreconditionRequired(
            "token exchange endpoint is not configured for this server".to_owned()
        )
    );
    assert!(exchanger.post.calls().is_empty());
}

#[tokio::test]
async fn empty_endpoint_is_precondition_required() {
    let post = RecordingPost::new(vec![]);
    let exchanger = exchanger(post, clock(1000));
    let mut config = base_config();
    config.token_exchange_endpoint = Some(String::new());
    let error = exchanger
        .exchange("caller-jwt", &server(), &config, "")
        .await
        .unwrap_err();
    assert!(matches!(error, CredError::PreconditionRequired(_)));
    assert!(exchanger.post.calls().is_empty());
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(String::new()))]
#[tokio::test]
async fn an_unusable_client_id_is_misconfigured(#[case] client_id: Option<String>) {
    let post = RecordingPost::new(vec![]);
    let exchanger = exchanger(post, clock(1000));
    let mut config = base_config();
    config.client_id = client_id;
    let error = exchanger
        .exchange("caller-jwt", &server(), &config, "")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::Misconfigured("token_exchange requires client_id and client_secret".to_owned())
    );
    assert!(exchanger.post.calls().is_empty());
}

#[tokio::test]
async fn a_missing_client_secret_is_misconfigured() {
    let post = RecordingPost::new(vec![]);
    let exchanger = exchanger(post, clock(1000));
    let mut config = base_config();
    config.client_secret = None;
    let error = exchanger
        .exchange("caller-jwt", &server(), &config, "")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::Misconfigured("token_exchange requires client_id and client_secret".to_owned())
    );
    assert!(exchanger.post.calls().is_empty());
}

#[tokio::test]
async fn an_empty_basic_auth_secret_is_misconfigured() {
    let post = RecordingPost::new(vec![]);
    let exchanger = exchanger(post, clock(1000));
    let mut config = base_config();
    config.client_secret = Some(SecretString::from(""));
    config.token_endpoint_auth_method = Some(TokenEndpointAuthMethod::ClientSecretBasic);
    let error = exchanger
        .exchange("caller-jwt", &server(), &config, "")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::Misconfigured(
            "token_endpoint_auth_method=client_secret_basic requires both client_id and client_secret"
                .to_owned()
        )
    );
    assert!(exchanger.post.calls().is_empty());
}

#[tokio::test]
async fn entra_obo_without_a_scope_is_misconfigured() {
    let post = RecordingPost::new(vec![]);
    let exchanger = exchanger(post, clock(1000));
    let mut config = base_config();
    config.profile = TokenExchangeProfile::EntraObo;
    let error = exchanger
        .exchange("caller-jwt", &server(), &config, "")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::Misconfigured(
            "entra_obo token exchange requires a scope (e.g. api://<app-id>/.default)".to_owned()
        )
    );
    assert!(exchanger.post.calls().is_empty());
}

#[test]
fn cache_key_is_the_sha256_of_the_joined_material() {
    assert_eq!(
        cache_key::cache_key("caller-jwt", "acme", &config()),
        "d7028d735be7bc336f2842b51ea8f1b2e342f18e9563b62978f6b3ff56cce25a"
    );
}

#[test]
fn cache_key_treats_empty_and_absent_optionals_alike() {
    let mut absent = config();
    absent.token_exchange_endpoint = None;
    absent.audience = None;
    absent.client_id = None;
    let mut empty = absent.clone();
    empty.token_exchange_endpoint = Some(String::new());
    empty.audience = Some(String::new());
    empty.client_id = Some(String::new());
    assert_eq!(
        cache_key::cache_key("jwt", "", &absent),
        cache_key::cache_key("jwt", "", &empty)
    );
}

#[derive(Clone, Copy)]
enum Change {
    Tenant,
    SubjectToken,
    Endpoint,
    Audience,
    SubjectType,
    ClientId,
    Secret,
    AuthMethod,
    Scopes,
    Profile,
}

#[rstest]
#[case::tenant(Change::Tenant)]
#[case::subject_token(Change::SubjectToken)]
#[case::endpoint(Change::Endpoint)]
#[case::audience(Change::Audience)]
#[case::subject_type(Change::SubjectType)]
#[case::client_id(Change::ClientId)]
#[case::secret(Change::Secret)]
#[case::auth_method(Change::AuthMethod)]
#[case::scopes(Change::Scopes)]
#[case::profile(Change::Profile)]
fn cache_key_changes_when_a_bound_value_changes(#[case] change: Change) {
    let baseline_config = config();
    let baseline = cache_key::cache_key("caller-jwt", "acme", &baseline_config);
    let mut mutated = baseline_config.clone();
    let changed = match change {
        Change::Tenant => cache_key::cache_key("caller-jwt", "globex", &baseline_config),
        Change::SubjectToken => cache_key::cache_key("other-jwt", "acme", &baseline_config),
        Change::Endpoint => {
            mutated.token_exchange_endpoint = Some("https://idp.example.com/other".to_owned());
            cache_key::cache_key("caller-jwt", "acme", &mutated)
        }
        Change::Audience => {
            mutated.audience = Some("https://other.example.com".to_owned());
            cache_key::cache_key("caller-jwt", "acme", &mutated)
        }
        Change::SubjectType => {
            mutated.subject_token_type = "urn:ietf:params:oauth:token-type:jwt".to_owned();
            cache_key::cache_key("caller-jwt", "acme", &mutated)
        }
        Change::ClientId => {
            mutated.client_id = Some("other-cid".to_owned());
            cache_key::cache_key("caller-jwt", "acme", &mutated)
        }
        Change::Secret => {
            mutated.client_secret = Some(SecretString::from("other-csec"));
            cache_key::cache_key("caller-jwt", "acme", &mutated)
        }
        Change::AuthMethod => {
            mutated.token_endpoint_auth_method = Some(TokenEndpointAuthMethod::ClientSecretBasic);
            cache_key::cache_key("caller-jwt", "acme", &mutated)
        }
        Change::Scopes => {
            mutated.scopes = vec!["s3".to_owned()];
            cache_key::cache_key("caller-jwt", "acme", &mutated)
        }
        Change::Profile => {
            mutated.profile = TokenExchangeProfile::EntraObo;
            cache_key::cache_key("caller-jwt", "acme", &mutated)
        }
    };
    assert_ne!(baseline, changed);
}

#[rstest]
#[case::missing("{}")]
#[case::empty(r#"{"access_token":""}"#)]
#[case::numeric(r#"{"access_token":7}"#)]
#[tokio::test]
async fn an_unusable_access_token_is_upstream_unavailable(#[case] raw: &str) {
    let post = RecordingPost::new(vec![Scripted::Body(body(raw))]);
    let exchanger = exchanger(post, clock(1000));
    let error = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::UpstreamUnavailable(
            "token exchange did not return a usable access token".to_owned()
        )
    );
}

#[rstest]
#[case::upper("Bearer")]
#[case::lower("bearer")]
#[case::mixed("BEARER")]
#[case::padded("  Bearer  ")]
#[tokio::test]
async fn a_bearer_token_type_is_accepted_in_any_case(#[case] token_type: &str) {
    let raw = format!(r#"{{"access_token":"x","token_type":"{token_type}"}}"#);
    let post = RecordingPost::new(vec![Scripted::Body(body(&raw))]);
    let exchanger = exchanger(post, clock(1000));
    let token = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(token.access_token, "x");
}

#[rstest]
#[case::n_a("N_A")]
#[case::lower_n_a("n_a")]
#[case::dpop("DPoP")]
#[case::mac("mac")]
#[tokio::test]
async fn a_non_bearer_token_type_is_refused(#[case] token_type: &str) {
    let raw = format!(r#"{{"access_token":"x","token_type":"{token_type}"}}"#);
    let post = RecordingPost::new(vec![Scripted::Body(body(&raw))]);
    let exchanger = exchanger(post, clock(1000));
    let error = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap_err();
    assert!(matches!(error, CredError::UpstreamUnavailable(_)));
}

#[tokio::test]
async fn an_absent_token_type_is_accepted() {
    let post = RecordingPost::new(vec![Scripted::Body(body(r#"{"access_token":"x"}"#))]);
    let exchanger = exchanger(post, clock(1000));
    let token = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(token.access_token, "x");
}

#[tokio::test]
async fn a_non_string_token_type_is_ignored() {
    let post = RecordingPost::new(vec![Scripted::Body(body(
        r#"{"access_token":"x","token_type":7}"#,
    ))]);
    let exchanger = exchanger(post, clock(1000));
    let token = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(token.access_token, "x");
}

#[rstest]
#[case::refresh("urn:ietf:params:oauth:token-type:refresh_token")]
#[case::id("urn:ietf:params:oauth:token-type:id_token")]
#[case::saml1("urn:ietf:params:oauth:token-type:saml1")]
#[case::saml2("urn:ietf:params:oauth:token-type:saml2")]
#[tokio::test]
async fn a_non_access_issued_token_type_is_refused(#[case] issued_type: &str) {
    let raw = format!(
        r#"{{"access_token":"x","token_type":"Bearer","issued_token_type":"{issued_type}"}}"#
    );
    let post = RecordingPost::new(vec![Scripted::Body(body(&raw))]);
    let exchanger = exchanger(post, clock(1000));
    let error = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap_err();
    assert!(matches!(error, CredError::UpstreamUnavailable(_)));
}

#[rstest]
#[case::access("urn:ietf:params:oauth:token-type:access_token")]
#[case::jwt("urn:ietf:params:oauth:token-type:jwt")]
#[case::custom("custom-unknown")]
#[tokio::test]
async fn another_issued_token_type_is_accepted(#[case] issued_type: &str) {
    let raw = format!(r#"{{"access_token":"x","issued_token_type":"{issued_type}"}}"#);
    let post = RecordingPost::new(vec![Scripted::Body(body(&raw))]);
    let exchanger = exchanger(post, clock(1000));
    let token = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(token.access_token, "x");
}

#[test]
fn a_non_string_issued_token_type_is_ignored() {
    let parsed = body(r#"{"access_token":"x","issued_token_type":7}"#);
    let token = super::body::token_from_body(&parsed, at(1000)).unwrap();
    assert_eq!(token.access_token, "x");
}

#[rstest]
#[case::integer("120", Some(120))]
#[case::float("120.0", Some(120))]
#[case::fraction("120.9", Some(120))]
#[case::negative("-5", Some(-5))]
#[case::numeric_string(r#""120""#, Some(120))]
#[case::float_string(r#""120.0""#, Some(120))]
#[case::padded_string(r#"" 120 ""#, Some(120))]
#[case::exponent_string(r#""1e2""#, Some(100))]
#[case::empty_string(r#""""#, None)]
#[case::text(r#""abc""#, None)]
#[case::boolean_true("true", None)]
#[case::boolean_false("false", None)]
#[case::null("null", None)]
#[case::array("[]", None)]
#[case::object("{}", None)]
fn parse_expires_in_reads_the_declared_forms(#[case] raw: &str, #[case] expected: Option<i64>) {
    let value: CredentialValue = serde_json::from_str(raw).unwrap();
    assert_eq!(parse_expires_in(Some(&value)), expected);
}

#[test]
fn a_missing_expires_in_is_absent() {
    assert_eq!(parse_expires_in(None), None);
}

#[tokio::test]
async fn expires_in_drives_the_expiry() {
    let post = RecordingPost::new(vec![Scripted::Body(body(
        r#"{"access_token":"x","expires_in":"120.0"}"#,
    ))]);
    let exchanger = exchanger(post, clock(1000));
    let token = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(token.expires_at, Some(at(1120)));
}

#[tokio::test]
async fn a_token_without_an_expiry_has_none() {
    let post = RecordingPost::new(vec![Scripted::Body(body(r#"{"access_token":"x"}"#))]);
    let exchanger = exchanger(post, clock(1000));
    let token = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(token.expires_at, None);
}

#[tokio::test]
async fn a_negative_expires_in_expires_immediately() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first","expires_in":-5}"#)),
        Scripted::Body(body(r#"{"access_token":"second","expires_in":-5}"#)),
    ]);
    let exchanger = exchanger(post, clock(1000));
    let first = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(first.expires_at, Some(at(995)));
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 2);
}

#[tokio::test]
async fn a_subject_rejection_maps_to_unauthorized_with_its_claims() {
    let post = RecordingPost::new(vec![Scripted::SubjectRejected {
        detail: "IdP rejected the subject token (HTTP 400)".to_owned(),
        claims: Some("{\"access_token\":{\"acrs\":{\"essential\":true}}}".to_owned()),
    }]);
    let exchanger = exchanger(post, clock(1000));
    let error = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::Unauthorized(Unauthorized {
            detail: "IdP rejected the subject token (HTTP 400)".to_owned(),
            www_authenticate: None,
            body: None,
            claims: Some("{\"access_token\":{\"acrs\":{\"essential\":true}}}".to_owned()),
        })
    );
}

#[tokio::test]
async fn a_subject_rejection_without_claims_still_maps_to_unauthorized() {
    let post = RecordingPost::new(vec![Scripted::SubjectRejected {
        detail: "rejected".to_owned(),
        claims: None,
    }]);
    let exchanger = exchanger(post, clock(1000));
    let error = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap_err();
    let CredError::Unauthorized(unauthorized) = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(unauthorized.detail, "rejected");
    assert_eq!(unauthorized.claims, None);
}

#[tokio::test]
async fn an_empty_rejection_detail_uses_the_default_message() {
    let post = RecordingPost::new(vec![Scripted::SubjectRejected {
        detail: String::new(),
        claims: None,
    }]);
    let exchanger = exchanger(post, clock(1000));
    let error = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap_err();
    let CredError::Unauthorized(unauthorized) = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(unauthorized.detail, "subject token rejected by the IdP");
}

#[tokio::test]
async fn a_gateway_fault_maps_to_misconfigured() {
    let post = RecordingPost::new(vec![Scripted::ClientFault {
        code: "invalid_client".to_owned(),
    }]);
    let exchanger = exchanger(post, clock(1000));
    let error = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::Misconfigured(
            "token exchange configuration error: the gateway's credentials, audience, or scope \
             for this server were not accepted by the IdP"
                .to_owned()
        )
    );
}

#[tokio::test]
async fn a_transport_failure_maps_to_upstream_unavailable() {
    let post = RecordingPost::new(vec![Scripted::Transport]);
    let exchanger = exchanger(post, clock(1000));
    let error = exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CredError::UpstreamUnavailable(
            "token exchange did not return a usable access token".to_owned()
        )
    );
}

#[tokio::test]
async fn an_expiring_token_is_cached_until_its_lifetime_minus_the_buffer() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first","expires_in":120}"#)),
        Scripted::Body(body(r#"{"access_token":"second","expires_in":120}"#)),
    ]);
    let clock = clock(1000);
    let exchanger = exchanger(post, Arc::clone(&clock));
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    clock.set(1059);
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 1);
    clock.set(1060);
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 2);
}

#[tokio::test]
async fn a_short_lived_token_is_not_cached_past_its_expiry() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first","expires_in":5}"#)),
        Scripted::Body(body(r#"{"access_token":"second","expires_in":5}"#)),
    ]);
    let clock = clock(1000);
    let exchanger = exchanger(post, Arc::clone(&clock));
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    clock.set(1004);
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 1);
    clock.set(1005);
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 2);
}

#[tokio::test]
async fn a_token_below_the_minimum_ttl_is_cached_for_the_floor_capped_at_its_lifetime() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first","expires_in":15}"#)),
        Scripted::Body(body(r#"{"access_token":"second","expires_in":15}"#)),
    ]);
    let clock = clock(1000);
    let exchanger = exchanger(post, Arc::clone(&clock));
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    clock.set(1009);
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 1);
    clock.set(1010);
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 2);
}

#[tokio::test]
async fn a_token_without_an_expiry_is_cached_for_the_default_ttl() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first"}"#)),
        Scripted::Body(body(r#"{"access_token":"second"}"#)),
    ]);
    let clock = clock(1000);
    let exchanger = exchanger(post, Arc::clone(&clock));
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    clock.set(4599);
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 1);
    clock.set(4600);
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 2);
}

#[tokio::test]
async fn a_zero_expires_in_re_exchanges_every_call() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first","expires_in":0}"#)),
        Scripted::Body(body(r#"{"access_token":"second","expires_in":0}"#)),
    ]);
    let exchanger = exchanger(post, clock(1000));
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 2);
}
