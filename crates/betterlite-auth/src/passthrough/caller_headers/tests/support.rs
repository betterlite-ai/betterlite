#![cfg(test)]

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_models::model_registration::JsonObject;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::dev_auth_mode::AuthModeFlags;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

use super::caller_filter::FilterSettings;

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

pub(super) fn settings<'settings>(
    master_key: Option<&'settings SecretString>,
    auth_mode: AuthModeFlags,
    custom_auth_configured: bool,
    litellm_key_header_name: Option<&'settings str>,
    endpoint_headers: &'settings [&'settings JsonObject],
) -> FilterSettings<'settings> {
    FilterSettings {
        master_key,
        auth_mode,
        custom_auth_configured,
        litellm_key_header_name,
        endpoint_headers,
    }
}

pub(super) fn caller(stored_api_key: Option<&str>, jwt_claims: Option<&str>) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    if let Some(stored) = stored_api_key {
        fields.insert("api_key".to_owned(), JsonData::String(stored.to_owned()));
    }
    if let Some(claims) = jwt_claims {
        let parsed = serde_json::from_str(claims).expect("the claim fixture is JSON");
        fields.insert("jwt_claims".to_owned(), parsed);
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the caller auth constructs")
}

pub(super) fn claims(claims_json: &str) -> Payload {
    serde_json::from_str(claims_json).expect("the claim fixture is a JSON object")
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).expect("the header name is valid");
        let header_value =
            HeaderValue::from_bytes(value.as_bytes()).expect("the header value is valid");
        map.insert(header_name, header_value);
    }
    map
}

pub(super) fn jwt(claims_json: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(claims_json);
    let signature = URL_SAFE_NO_PAD.encode("signature");
    format!("{header}.{payload}.{signature}")
}

pub(super) fn sorted_names(forwarded: &IndexMap<String, SecretString>) -> Vec<&str> {
    let mut names: Vec<&str> = forwarded.keys().map(String::as_str).collect();
    names.sort_unstable();
    names
}

pub(super) fn forwarded_value<'forwarded>(
    forwarded: &'forwarded IndexMap<String, SecretString>,
    name: &str,
) -> Option<&'forwarded str> {
    forwarded.get(name).map(ExposeSecret::expose_secret)
}

pub(super) fn texts(names: &[String]) -> Vec<&str> {
    names.iter().map(String::as_str).collect()
}
