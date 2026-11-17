use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_config::sso_claims::{ClaimAttributes, load_claim_attributes};

use crate::jwt_claims::TypedValue;
use crate::jwt_claims::handler::JwtHandler;
use crate::jwt_claims::settings::JwtAuthSettings;
use crate::sso_types::identity::OpenIdIdentity;
use crate::sso_types::records::RoleMappings;
use crate::user_roles::UserRole;

pub(super) fn document(raw: &str) -> TypedValue {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn claims(raw: &str) -> BTreeMap<String, TypedValue> {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn segment(raw: &str) -> String {
    URL_SAFE_NO_PAD.encode(raw.as_bytes())
}

pub(super) fn token(payload: &str) -> String {
    format!(
        "{}.{}.{}",
        segment(r#"{"alg":"HS256","typ":"JWT"}"#),
        segment(payload),
        segment("signature")
    )
}

pub(super) fn handler(settings: JwtAuthSettings) -> JwtHandler {
    let mut handler = JwtHandler::default();
    handler.update_environment(settings, Vec::new(), 0);
    handler
}

pub(super) fn attributes(pairs: &[(&str, &str)]) -> ClaimAttributes {
    let environment = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<String, String>>();
    load_claim_attributes(&environment)
}

pub(super) fn identity() -> OpenIdIdentity {
    OpenIdIdentity {
        id: Some("user-1".to_owned()),
        email: Some("User@Example.com".to_owned()),
        first_name: None,
        last_name: None,
        display_name: None,
        picture: None,
        provider: None,
        team_ids: Vec::new(),
        user_role: None,
        extra_fields: None,
    }
}

pub(super) fn mappings(
    default_role: Option<UserRole>,
    roles: &[(UserRole, &[&str])],
) -> RoleMappings {
    RoleMappings {
        provider: "generic".to_owned(),
        group_claim: "groups".to_owned(),
        default_role,
        roles: roles
            .iter()
            .map(|(role, groups)| {
                (
                    *role,
                    groups.iter().map(|group| (*group).to_owned()).collect(),
                )
            })
            .collect(),
    }
}
