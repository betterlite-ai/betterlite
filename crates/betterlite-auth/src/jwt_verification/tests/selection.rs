use betterlite_config::sections::jwt_settings::Settings as JwtAuthSettings;
use betterlite_config::sections::jwt_settings::records::{IssuerConfig, Selector};
use betterlite_models::decisions::Json;

use crate::jwt_verification::keys::PublicKey;
use crate::jwt_verification::settings::DecodeOptions;
use crate::jwt_verification::verifier::JwtVerifier;

use super::fixtures;
use super::support::{
    ScriptedSource, SourceReply, audience_one, capture_logs, claim_fields, harness, issuer_config,
    jwt_settings_with_issuers, lock, parse_json, settings, text_claim,
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

#[tokio::test]
async fn first_exact_issuer_or_global_fallback() {
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
    let h = harness(settings(
        Some("litellm-proxy"),
        None,
        jwt_settings_with_issuers(vec![first, second]),
    ));
    let source =
        ScriptedSource::with_url(first_url, SourceReply::Key(fixtures::RSA_JWK.to_owned()));
    lock(&source.global).push_back(SourceReply::Key(fixtures::RSA_JWK.to_owned()));
    let payload = h
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

    let fallback = h
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

#[test]
fn managed_trust_first_match_requires_scoped_audience() {
    let issuer = Json::Text(fixtures::ISSUER_A.to_owned());
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
        &issuer
    ));
    assert!(trust_of(
        vec![with_audience(
            issuer_config(fixtures::ISSUER_A),
            audience_one("litellm-proxy")
        )],
        None,
        None,
        &issuer
    ));
    let mut disabled = with_audience(
        issuer_config(fixtures::ISSUER_A),
        audience_one("litellm-proxy"),
    );
    disabled.disable_audience_validation = true;
    assert!(!trust_of(vec![disabled], None, None, &issuer));
    assert!(!trust_of(
        vec![with_audience(
            issuer_config(fixtures::ISSUER_A),
            Some(Selector::Many(Vec::new()))
        )],
        None,
        None,
        &issuer
    ));
    assert!(trust_of(
        vec![with_audience(
            issuer_config(fixtures::ISSUER_A),
            Some(Selector::Many(vec!["litellm-proxy".to_owned()]))
        )],
        None,
        None,
        &issuer
    ));
    assert!(!trust_of(Vec::new(), None, None, &issuer));
    assert!(!trust_of(Vec::new(), None, None, &parse_json("1")));
    assert!(!trust_of(Vec::new(), None, None, &parse_json("null")));
    assert!(trust_of(
        vec![issuer_config("https://other.example.com")],
        Some("litellm-proxy"),
        Some(fixtures::ISSUER_A),
        &issuer
    ));
    assert!(!trust_of(
        vec![issuer_config("https://other.example.com")],
        Some(""),
        Some(fixtures::ISSUER_A),
        &issuer
    ));
    assert!(!trust_of(
        Vec::new(),
        Some("litellm-proxy"),
        Some(fixtures::ISSUER_A),
        &Json::Text("https://other.example.com".to_owned())
    ));
}

#[tokio::test]
async fn empty_env_is_not_absent() {
    let (logs, _guard) = capture_logs();
    let absent = harness(settings(None, None, JwtAuthSettings::default()));
    let absent_kwargs = absent.verifier.build_decode_kwargs();
    assert_eq!(
        absent_kwargs.options,
        Some(DecodeOptions {
            verify_aud: Some(false),
            verify_iss: Some(false),
        })
    );
    assert_eq!(logs.warnings(), 1);

    let empty = harness(settings(Some(""), Some(""), JwtAuthSettings::default()));
    let empty_kwargs = empty.verifier.build_decode_kwargs();
    assert_eq!(empty_kwargs.audience.as_deref(), Some(""));
    assert_eq!(empty_kwargs.issuer.as_deref(), Some(""));
    assert_eq!(empty_kwargs.options, None);
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

#[test]
fn explicit_empty_options_bypasses_builder() {
    let h = harness(settings(None, None, JwtAuthSettings::default()));
    let jwk = parse_json(fixtures::RSA_JWK);
    let key = PublicKey::Jwk(&jwk);
    let builder_error = JwtVerifier::decode_options(None, None, false).unwrap_err();
    assert_eq!(
        builder_error.to_string(),
        "Validation fails: audience must be provided unless disable_audience_validation=True"
    );
    let explicit = Some(DecodeOptions {
        verify_aud: None,
        verify_iss: None,
    });
    let payload = h
        .verifier
        .decode_with_public_key(fixtures::TOKEN_UNSCOPED, &key, None, None, explicit, false)
        .unwrap();
    assert_eq!(text_claim(&claim_fields(&payload), "sub"), Some("user-1"));
    let error = h
        .verifier
        .decode_with_public_key(fixtures::TOKEN_RS256, &key, None, None, explicit, false)
        .unwrap_err();
    assert_eq!(error.to_string(), "Validation fails: Invalid audience");
}

#[test]
fn disable_flag_does_not_disable_present_audience() {
    let audience = Some(Selector::One("litellm-proxy".to_owned()));
    let kept = JwtVerifier::decode_options(audience.as_ref(), None, true).unwrap();
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
    let h = harness(settings(None, None, JwtAuthSettings::default()));
    let jwk = parse_json(fixtures::RSA_JWK);
    let key = PublicKey::Jwk(&jwk);
    let payload = h
        .verifier
        .decode_with_public_key(
            fixtures::TOKEN_RS256,
            &key,
            audience.as_ref(),
            None,
            None,
            true,
        )
        .unwrap();
    assert_eq!(text_claim(&claim_fields(&payload), "sub"), Some("user-1"));
    let error = h
        .verifier
        .decode_with_public_key(
            fixtures::TOKEN_WRONG_AUDIENCE,
            &key,
            audience.as_ref(),
            None,
            None,
            true,
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "Validation fails: Invalid audience");
}

#[tokio::test]
async fn unscoped_warning_once() {
    let (logs, _guard) = capture_logs();
    let h = harness(settings(None, None, JwtAuthSettings::default()));
    let source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    h.verifier
        .auth_jwt(fixtures::TOKEN_RS256, &source)
        .await
        .unwrap();
    let second_source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    h.verifier
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

#[tokio::test]
async fn unmatched_issuer_fallback() {
    let (logs, _guard) = capture_logs();
    let jwt = jwt_settings_with_issuers(vec![with_audience(
        issuer_config(fixtures::ISSUER_A),
        Some(Selector::One("litellm-proxy".to_owned())),
    )]);
    let h = harness(settings(None, None, jwt));
    let source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let payload = h
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
