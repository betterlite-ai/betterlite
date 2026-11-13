use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::jwt_claims::TypedValue;
use crate::jwt_claims::claims::JwtClaims;
use crate::jwt_claims::settings::JwtAuthSettings;

pub(super) fn document(raw: &str) -> TypedValue {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn empty_settings() -> JwtAuthSettings {
    JwtAuthSettings {
        team_id_jwt_field: None,
        team_ids_jwt_field: None,
        team_id_default: None,
        team_alias_jwt_field: None,
        org_id_jwt_field: None,
        org_alias_jwt_field: None,
        user_id_jwt_field: None,
        user_email_jwt_field: None,
        user_allowed_email_domain: None,
        user_id_upsert: false,
        end_user_id_jwt_field: None,
        agent_id_jwt_field: None,
        object_id_jwt_field: None,
    }
}

pub(super) fn project<'auth>(
    claims_raw: &str,
    settings: &'auth JwtAuthSettings,
    issuers: &'auth [String],
) -> JwtClaims<'auth> {
    JwtClaims {
        claims: document(claims_raw),
        settings,
        trusted_issuers: issuers,
    }
}

pub(super) fn segment(raw: &str) -> String {
    URL_SAFE_NO_PAD.encode(raw.as_bytes())
}

pub(super) fn token(header: &str, payload: &str, signature: &str) -> String {
    format!(
        "{}.{}.{}",
        segment(header),
        segment(payload),
        segment(signature)
    )
}

pub(super) fn issuers(configured: &[&str]) -> Vec<String> {
    configured
        .iter()
        .map(|issuer| (*issuer).to_owned())
        .collect()
}
