use betterlite_config::sections::jwt_settings::Settings as JwtAuthSettings;
use betterlite_config::sections::jwt_settings::records::IssuerConfig;

use crate::jwt_verification::normalize::strip_internal_claims;
use crate::passthrough::caller_headers::header_sets::LITELLM_INTERNAL_CLAIMS;

use super::fixtures;
use super::support::{
    Harness, ScriptedSource, SourceReply, audience_one, claim_fields, document, harness,
    issuer_config, jwt_settings_with_issuers, parse_json, settings, text_claim,
};

const DISCOVERY: &str = "https://issuer-a.example.com/.well-known/openid-configuration";

fn issuer_harness(config: IssuerConfig) -> Harness {
    let jwt = jwt_settings_with_issuers(vec![config]);
    harness(settings(None, None, jwt))
}

fn mapped_config() -> IssuerConfig {
    let mut config = issuer_config(fixtures::ISSUER_A);
    config.audience = audience_one("litellm-proxy");
    config.user_id_jwt_field = Some("user.id".to_owned());
    config.user_email_jwt_field = Some("user.email".to_owned());
    config.team_id_jwt_field = Some("team.id".to_owned());
    config.team_ids_jwt_field = Some("teams".to_owned());
    config.org_id_jwt_field = Some("org.id".to_owned());
    config.end_user_id_jwt_field = Some("end_user.id".to_owned());
    config
}

#[test]
fn exactly_seven_internal_claims_removed() {
    assert_eq!(LITELLM_INTERNAL_CLAIMS.len(), 7);
    let payload = document(
        r#"{"sub":"u","_litellm_user_id":"a","_litellm_user_email":"b","_litellm_team_id":"c","_litellm_team_ids":["d"],"_litellm_org_id":"e","_litellm_end_user_id":"f","_litellm_jwt_issuer":"g","_litellm_other":"kept"}"#,
    );
    let stripped = strip_internal_claims(&payload).unwrap();
    let fields = claim_fields(&stripped);
    for name in LITELLM_INTERNAL_CLAIMS {
        assert!(!fields.contains_key(name), "the {name} claim survived");
    }
    assert_eq!(text_claim(&fields, "_litellm_other"), Some("kept"));
    assert_eq!(text_claim(&fields, "sub"), Some("u"));
    assert_eq!(fields.len(), 2);
}

#[tokio::test]
async fn injected_litellm_team_id_stripped() {
    let h = harness(settings(
        Some("litellm-proxy"),
        None,
        JwtAuthSettings::default(),
    ));
    let source = ScriptedSource::with_keys(fixtures::RSA_JWK);
    let payload = h
        .verifier
        .auth_jwt(fixtures::TOKEN_INJECTED_CLAIMS, &source)
        .await
        .unwrap();
    let fields = claim_fields(&payload);
    for name in LITELLM_INTERNAL_CLAIMS {
        assert!(!fields.contains_key(name), "the {name} claim survived");
    }
    assert_eq!(text_claim(&fields, "_litellm_other"), Some("kept"));
}

#[tokio::test]
async fn issuer_claim_map() {
    let h = issuer_harness(mapped_config());
    let source =
        ScriptedSource::with_url(DISCOVERY, SourceReply::Key(fixtures::RSA_JWK.to_owned()));
    let payload = h
        .verifier
        .auth_jwt(fixtures::TOKEN_CLAIM_MAP, &source)
        .await
        .unwrap();
    let fields = claim_fields(&payload);
    assert_eq!(
        text_claim(&fields, "_litellm_jwt_issuer"),
        Some(fixtures::ISSUER_A)
    );
    assert_eq!(text_claim(&fields, "_litellm_user_id"), Some("u-1"));
    assert_eq!(
        text_claim(&fields, "_litellm_user_email"),
        Some("user@example.com")
    );
    assert_eq!(text_claim(&fields, "_litellm_team_id"), Some("t-1"));
    assert_eq!(text_claim(&fields, "_litellm_org_id"), Some("o-1"));
    assert_eq!(text_claim(&fields, "_litellm_end_user_id"), Some("e-1"));
    assert_eq!(
        fields.get("_litellm_team_ids"),
        Some(&parse_json("[\"t-1\",\"t-2\"]"))
    );
    assert_eq!(text_claim(&fields, "iss"), Some(fixtures::ISSUER_A));
}

#[tokio::test]
async fn issuer_mapping_missing_empty_false_zero_collections() {
    let mut config = issuer_config(fixtures::ISSUER_A);
    config.audience = audience_one("litellm-proxy");
    config.user_id_jwt_field = Some("profile.absent".to_owned());
    config.user_email_jwt_field = Some("profile.empty_text".to_owned());
    config.team_id_jwt_field = Some("profile.flag".to_owned());
    config.team_ids_jwt_field = Some("profile.empty_list".to_owned());
    config.org_id_jwt_field = Some("profile.empty_object".to_owned());
    config.end_user_id_jwt_field = Some("profile.count".to_owned());
    let h = issuer_harness(config);
    let source =
        ScriptedSource::with_url(DISCOVERY, SourceReply::Key(fixtures::RSA_JWK.to_owned()));
    let payload = h
        .verifier
        .auth_jwt(fixtures::TOKEN_CLAIM_MAP, &source)
        .await
        .unwrap();
    let fields = claim_fields(&payload);
    assert!(!fields.contains_key("_litellm_user_id"));
    assert!(!fields.contains_key("_litellm_user_email"));
    assert_eq!(fields.get("_litellm_team_id"), Some(&parse_json("false")));
    assert_eq!(fields.get("_litellm_team_ids"), Some(&parse_json("[]")));
    assert_eq!(fields.get("_litellm_org_id"), Some(&parse_json("{}")));
    assert_eq!(fields.get("_litellm_end_user_id"), Some(&parse_json("0")));

    let mut null_config = issuer_config(fixtures::ISSUER_A);
    null_config.audience = audience_one("litellm-proxy");
    null_config.user_id_jwt_field = Some("profile.null_value".to_owned());
    let null_h = issuer_harness(null_config);
    let null_source =
        ScriptedSource::with_url(DISCOVERY, SourceReply::Key(fixtures::RSA_JWK.to_owned()));
    let null_payload = null_h
        .verifier
        .auth_jwt(fixtures::TOKEN_CLAIM_MAP, &null_source)
        .await
        .unwrap();
    assert!(!claim_fields(&null_payload).contains_key("_litellm_user_id"));
}

#[tokio::test]
async fn mappings_read_original_payload() {
    let mut config = issuer_config(fixtures::ISSUER_A);
    config.audience = audience_one("litellm-proxy");
    config.user_id_jwt_field = Some("_litellm_team_id".to_owned());
    config.team_id_jwt_field = Some("_litellm_user_id".to_owned());
    config.user_email_jwt_field = Some("nested.id".to_owned());
    config.team_ids_jwt_field = Some("_litellm_team_ids".to_owned());
    let h = issuer_harness(config);
    let source =
        ScriptedSource::with_url(DISCOVERY, SourceReply::Key(fixtures::RSA_JWK.to_owned()));
    let payload = h
        .verifier
        .auth_jwt(fixtures::TOKEN_INJECTED_ISSUER, &source)
        .await
        .unwrap();
    let fields = claim_fields(&payload);
    assert_eq!(
        text_claim(&fields, "_litellm_user_id"),
        Some("attacker-team")
    );
    assert_eq!(
        text_claim(&fields, "_litellm_team_id"),
        Some("attacker-user")
    );
    assert_eq!(
        text_claim(&fields, "_litellm_user_email"),
        Some("nested-id")
    );
    assert_eq!(
        fields.get("_litellm_team_ids"),
        Some(&parse_json("[\"attacker-team\"]"))
    );
    assert_eq!(
        text_claim(&fields, "_litellm_jwt_issuer"),
        Some(fixtures::ISSUER_A)
    );
    assert!(!fields.contains_key("_litellm_org_id"));
}
