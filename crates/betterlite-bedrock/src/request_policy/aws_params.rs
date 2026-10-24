use secrecy::ExposeSecret as _;

use crate::request_policy::{RequestValue, lookup, upsert};

const AUTH_KEYS: [&str; 11] = [
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_session_token",
    "aws_session_name",
    "aws_profile_name",
    "aws_role_name",
    "aws_web_identity_token",
    "aws_sts_endpoint",
    "aws_external_id",
    "aws_session_tags",
    "aws_region_name",
];
const STATIC_KEYS: [&str; 3] = [
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_region_name",
];

#[must_use]
pub fn merge_aws_request_params(
    deployment: &[(String, RequestValue)],
    request: &[(String, RequestValue)],
) -> Vec<(String, RequestValue)> {
    let mut merged = request.to_vec();
    for (name, value) in deployment {
        upsert(&mut merged, name, value.clone());
    }
    let static_credentials = STATIC_KEYS.iter().all(|key| match lookup(deployment, key) {
        Some(RequestValue::Text(text)) => !text.is_empty(),
        Some(RequestValue::Secret(secret)) => !secret.expose_secret().is_empty(),
        Some(RequestValue::Mapping(_) | RequestValue::Raw(_)) | None => false,
    });
    // a deployment with static keys must not let the caller pick another identity the server holds
    if static_credentials {
        merged.retain(|(name, _)| {
            !AUTH_KEYS.contains(&name.as_str()) || lookup(deployment, name).is_some()
        });
    }
    merged
}

pub fn map_auth_aliases(
    non_default: &[(String, RequestValue)],
    optional: &mut Vec<(String, RequestValue)>,
) {
    for (param, value) in non_default {
        if param == "region_name" {
            upsert(optional, "aws_region_name", value.clone());
        }
    }
}
