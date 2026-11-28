use betterlite_config::sections::jwt_settings::Settings as JwtAuthSettings;
use betterlite_config::sections::jwt_settings::records::{IssuerConfig, Selector};
use betterlite_models::decisions::Json;

use crate::jwt_verification::keys::PublicKey;
use crate::jwt_verification::settings::DecodeOptions;
use crate::jwt_verification::verifier::{DecodeParams, JwtVerifier};

use super::fixtures;
use super::support::{
    CapturedLogs, Harness, ScriptedSource, SourceReply, audience_one, capture_logs, claim_fields,
    harness, issuer_config, jwt_settings_with_issuers, lock, parse_json, settings, text_claim,
};

fn with_audience(mut config: IssuerConfig, audience: Option<Selector>) -> IssuerConfig {
    config.audience = audience;
    config
}

fn trust_of(
    configs: Vec<IssuerConfig>,
    env_audience: Option<&str>,
    env_issuer: Option<&str>,
    issuer: &Json,
) -> bool {
    let jwt = jwt_settings_with_issuers(configs);
    let verifier = harness(settings(env_audience, env_issuer, jwt));
    verifier.verifier.managed_issuer_is_trusted(issuer)
}

fn two_issuer_configs() -> (IssuerConfig, IssuerConfig, &'static str) {
    let first_url = "https://issuer-a.example.com/first-jwks";
    let mut first = with_audience(
        issuer_config(fixtures::ISSUER_A),
        Some(Selector::One("litellm-proxy".to_owned())),
    );
    first.jwks_url = Some(first_url.to_owned());
    first.user_id_jwt_field = Some("sub".to_owned());
    let mut second = with_audience(
        issuer_config(fixtures::ISSUER_A),
        Some(Selector::One("litellm-proxy".to_owned())),
    );
    second.jwks_url = Some("https://issuer-a.example.com/second-jwks".to_owned());
    second.user_email_jwt_field = Some("sub".to_owned());
    (first, second, first_url)
}

#[tokio::test]
async fn first_exact_issuer_or_global_fallback() {
    let (first, second, first_url) = two_issuer_configs();
    let harness = harness(settings(
        Some("litellm-proxy"),
        None,
        jwt_settings_with_issuers(vec![first, second]),
    ));
    let source =
        ScriptedSource::with_url(first_url, SourceReply::Key(fixtures::RSA_JWK.to_owned()));
    lock(&source.global).push_back(SourceReply::Key(fixtures::RSA_JWK.to_owned()));
    let payload = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_ISSUER_AUDIENCE, &source)
        .await
        .unwrap();
    let fields = claim_fields(&payload);
    assert_eq!(text_claim(&fields, "sub"), Some("user-1"));
    assert_eq!(text_claim(&fields, "_litellm_user_id"), Some("user-1"));
    assert_eq!(
        text_claim(&fields, "_litellm_jwt_issuer"),
        Some(fixtures::ISSUER_A)
    );
    assert!(!fields.contains_key("_litellm_user_email"));

    let fallback = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_WRONG_ISSUER, &source)
        .await
        .unwrap();
    let fallback_fields = claim_fields(&fallback);
    assert_eq!(
        text_claim(&fallback_fields, "iss"),
        Some("https://evil.example.com")
    );
    assert!(!fallback_fields.contains_key("_litellm_jwt_issuer"));
    assert_eq!(
        source.calls(),
        vec![format!("issuer:{first_url}"), "lookup:rsa-key".to_owned()]
    );
}

#[test]
fn explicit_jwks_or_trimmed_discovery() {
    let mut config = issuer_config(fixtures::ISSUER_A);
    config.jwks_url = Some("https://idp.example.com/keys".to_owned());
    assert_eq!(
        JwtVerifier::jwks_url_for_issuer(&config),
        "https://idp.example.com/keys"
    );
    config.jwks_url = Some(String::new());
    assert_eq!(
        JwtVerifier::jwks_url_for_issuer(&config),
        "https://issuer-a.example.com/.well-known/openid-configuration"
    );
    config.jwks_url = None;
    config.issuer = "https://idp.example.com///".to_owned();
    assert_eq!(
        JwtVerifier::jwks_url_for_issuer(&config),
        "https://idp.example.com/.well-known/openid-configuration"
    );
}

fn first_matching_issuer_decides(issuer: &Json) {
    assert!(!trust_of(
        vec![
            with_audience(issuer_config(fixtures::ISSUER_A), None),
            with_audience(
                issuer_config(fixtures::ISSUER_A),
                audience_one("litellm-proxy")
            ),
        ],
        None,
        None,
        issuer
    ));
    assert!(trust_of(
        vec![with_audience(
            issuer_config(fixtures::ISSUER_A),
            audience_one("litellm-proxy")
        )],
        None,
        None,
        issuer
    ));
}

fn audience_selector_details(issuer: &Json) {
    let mut disabled = with_audience(
        issuer_config(fixtures::ISSUER_A),
        audience_one("litellm-proxy"),
    );
    disabled.disable_audience_validation = true;
    assert!(!trust_of(vec![disabled], None, None, issuer));
    assert!(!trust_of(
        vec![with_audience(
            issuer_config(fixtures::ISSUER_A),
            Some(Selector::Many(Vec::new()))
        )],
        None,
        None,
        issuer
    ));
    assert!(trust_of(
        vec![with_audience(
            issuer_config(fixtures::ISSUER_A),
            Some(Selector::Many(vec!["litellm-proxy".to_owned()]))
        )],
        None,
        None,
        issuer
    ));
}

fn env_pair_is_the_fallback(issuer: &Json) {
    assert!(!trust_of(Vec::new(), None, None, issuer));
    assert!(!trust_of(Vec::new(), None, None, &parse_json("1")));
    assert!(!trust_of(Vec::new(), None, None, &parse_json("null")));
    assert!(trust_of(
        vec![issuer_config("https://other.example.com")],
        Some("litellm-proxy"),
        Some(fixtures::ISSUER_A),
        issuer
    ));
    assert!(!trust_of(
        vec![issuer_config("https://other.example.com")],
        Some(""),
        Some(fixtures::ISSUER_A),
        issuer
    ));
    assert!(!trust_of(
        Vec::new(),
        Some("litellm-proxy"),
        Some(fixtures::ISSUER_A),
        &Json::Text("https://other.example.com".to_owned())
    ));
}

#[test]
fn managed_trust_first_match_requires_scoped_audience() {
    let issuer = Json::Text(fixtures::ISSUER_A.to_owned());
    first_matching_issuer_decides(&issuer);
    audience_selector_details(&issuer);
    env_pair_is_the_fallback(&issuer);
}

async fn absent_env_details(absent: &Harness, logs: &CapturedLogs) {
    let absent_kwargs = absent.verifier.build_decode_kwargs();
    assert_eq!(
        absent_kwargs.options,
        Some(DecodeOptions {
            verify_aud: Some(false),
            verify_iss: Some(false),
        })
    );
    assert_eq!(logs.warnings(), 1);
    let absent_source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let absent_payload = absent
        .verifier
        .auth_jwt(fixtures::TOKEN_UNSCOPED, &absent_source)
        .await
        .unwrap();
    assert_eq!(
        text_claim(&claim_fields(&absent_payload), "sub"),
        Some("user-1")
    );
}

async fn empty_env_details(empty: &Harness, logs: &CapturedLogs) {
    let empty_kwargs = empty.verifier.build_decode_kwargs();
    assert_eq!(empty_kwargs.audience.as_deref(), Some(""));
    assert_eq!(empty_kwargs.issuer.as_deref(), Some(""));
    assert_eq!(empty_kwargs.options, None);
    assert_eq!(logs.warnings(), 1);
    let empty_source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let empty_error = empty
        .verifier
        .auth_jwt(fixtures::TOKEN_UNSCOPED, &empty_source)
        .await
        .unwrap_err();
    assert_eq!(
        empty_error.to_string(),
        "Validation fails: Token is missing the \"iss\" claim"
    );

    let audience_only = harness(settings(Some(""), None, JwtAuthSettings::default()));
    let audience_source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let audience_error = audience_only
        .verifier
        .auth_jwt(fixtures::TOKEN_UNSCOPED, &audience_source)
        .await
        .unwrap_err();
    assert_eq!(
        audience_error.to_string(),
        "Validation fails: Token is missing the \"aud\" claim"
    );
    assert_eq!(logs.warnings(), 1);
}

#[tokio::test]
async fn empty_env_is_not_absent() {
    let (logs, _guard) = capture_logs();
    let absent = harness(settings(None, None, JwtAuthSettings::default()));
    absent_env_details(&absent, &logs).await;
    let empty = harness(settings(Some(""), Some(""), JwtAuthSettings::default()));
    empty_env_details(&empty, &logs).await;
}

#[test]
fn explicit_empty_options_bypasses_builder() {
    let harness = harness(settings(None, None, JwtAuthSettings::default()));
    let jwk = parse_json(fixtures::RSA_JWK);
    let key = PublicKey::Jwk(&jwk);
    let builder_error = JwtVerifier::decode_options(None, None, false).unwrap_err();
    assert_eq!(
        builder_error.to_string(),
        "Validation fails: audience must be provided unless disable_audience_validation=True"
    );
    let params = DecodeParams {
        audience: None,
        issuer: None,
        options: Some(DecodeOptions {
            verify_aud: None,
            verify_iss: None,
        }),
        disable_audience_validation: false,
    };
    let payload = harness
        .verifier
        .decode_with_public_key(fixtures::TOKEN_UNSCOPED, &key, &params)
        .unwrap();
    assert_eq!(text_claim(&claim_fields(&payload), "sub"), Some("user-1"));
    let error = harness
        .verifier
        .decode_with_public_key(fixtures::TOKEN_RS256, &key, &params)
        .unwrap_err();
    assert_eq!(error.to_string(), "Validation fails: Invalid audience");
}

fn decode_options_with_disable_flag(audience: Option<&Selector>) {
    let kept = JwtVerifier::decode_options(audience, None, true).unwrap();
    assert_eq!(
        kept,
        Some(DecodeOptions {
            verify_aud: None,
            verify_iss: Some(false),
        })
    );
    let disabled = JwtVerifier::decode_options(None, None, true).unwrap();
    assert_eq!(
        disabled,
        Some(DecodeOptions {
            verify_aud: Some(false),
            verify_iss: Some(false),
        })
    );
}

#[test]
fn disable_flag_does_not_disable_present_audience() {
    let audience = Some(Selector::One("litellm-proxy".to_owned()));
    decode_options_with_disable_flag(audience.as_ref());
    let harness = harness(settings(None, None, JwtAuthSettings::default()));
    let jwk = parse_json(fixtures::RSA_JWK);
    let key = PublicKey::Jwk(&jwk);
    let params = DecodeParams {
        audience: audience.as_ref(),
        issuer: None,
        options: None,
        disable_audience_validation: true,
    };
    let payload = harness
        .verifier
        .decode_with_public_key(fixtures::TOKEN_RS256, &key, &params)
        .unwrap();
    assert_eq!(text_claim(&claim_fields(&payload), "sub"), Some("user-1"));
    let error = harness
        .verifier
        .decode_with_public_key(fixtures::TOKEN_WRONG_AUDIENCE, &key, &params)
        .unwrap_err();
    assert_eq!(error.to_string(), "Validation fails: Invalid audience");
}

#[tokio::test]
async fn unscoped_warning_once() {
    let (logs, _guard) = capture_logs();
    let harness = harness(settings(None, None, JwtAuthSettings::default()));
    let source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    harness
        .verifier
        .auth_jwt(fixtures::TOKEN_RS256, &source)
        .await
        .unwrap();
    let second_source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    harness
        .verifier
        .auth_jwt(fixtures::TOKEN_RS256, &second_source)
        .await
        .unwrap();
    assert_eq!(logs.warnings(), 1);
    let lines = logs.take();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("neither JWT_AUDIENCE nor JWT_ISSUER"))
    );
}

#[test]
fn audience_only_scope_stays_quiet_and_builds_partial_options() {
    let (logs, _guard) = capture_logs();
    let harness = harness(settings(
        Some("litellm-proxy"),
        None,
        JwtAuthSettings::default(),
    ));
    let kwargs = harness.verifier.build_decode_kwargs();
    assert_eq!(
        kwargs.options,
        Some(DecodeOptions {
            verify_aud: None,
            verify_iss: Some(false),
        })
    );
    assert_eq!(logs.warnings(), 0);
}

#[tokio::test]
async fn unmatched_issuer_fallback() {
    let (logs, _guard) = capture_logs();
    let jwt = jwt_settings_with_issuers(vec![with_audience(
        issuer_config(fixtures::ISSUER_A),
        Some(Selector::One("litellm-proxy".to_owned())),
    )]);
    let harness = harness(settings(None, None, jwt));
    let source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let payload = harness
        .verifier
        .auth_jwt(fixtures::TOKEN_WRONG_ISSUER, &source)
        .await
        .unwrap();
    let fields = claim_fields(&payload);
    assert_eq!(text_claim(&fields, "iss"), Some("https://evil.example.com"));
    assert!(!fields.contains_key("_litellm_jwt_issuer"));
    assert_eq!(source.calls(), vec!["lookup:rsa-key".to_owned()]);
    assert_eq!(logs.warnings(), 1);
}
