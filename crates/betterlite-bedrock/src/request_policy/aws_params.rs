use secrecy::ExposeSecret as _;

use crate::request_policy::{Params, RequestValue};

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
pub fn merge_aws_request_params(deployment: &Params, request: &Params) -> Params {
    let mut merged = request.clone();
    merged.extend(deployment.clone());
    let static_credentials = STATIC_KEYS.iter().all(|key| match deployment.get(*key) {
        Some(RequestValue::Text(text)) => !text.is_empty(),
        Some(RequestValue::Secret(secret)) => !secret.expose_secret().is_empty(),
        Some(RequestValue::Mapping(_) | RequestValue::Raw(_)) | None => false,
    });
    // a deployment with static keys must not let the caller pick another identity the server holds
    if static_credentials {
        merged
            .retain(|name, _| !AUTH_KEYS.contains(&name.as_str()) || deployment.contains_key(name));
    }
    merged
}

pub fn map_auth_aliases(non_default: &Params, optional: &mut Params) {
    for (param, value) in non_default {
        if param == "region_name" {
            optional.insert("aws_region_name".to_owned(), value.clone());
        }
    }
}
