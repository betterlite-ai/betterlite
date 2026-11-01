#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use secrecy::ExposeSecret;

use super::auth::{AuthInput, UserAPIKeyAuth};
use super::credential;
use super::view::VerificationTokenView;

fn object(text: &str) -> BTreeMap<String, JsonData> {
    match serde_json::from_str(text).unwrap_or_else(|error| panic!("invalid JSON fixture: {error}"))
    {
        JsonData::Object(fields) => fields,
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => panic!("fixture must be a JSON object"),
    }
}

#[test]
fn resolved_key_negative_numbers_kept() {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(object(
        "{\"team_max_budget\": -5.0, \"team_rpm_limit\": -10, \"user_max_budget\": -2.5}",
    )))
    .unwrap_or_else(|error| panic!("negative budgets should construct: {error}"));
    assert_eq!(auth.view.team_max_budget.unwrap().to_string(), "-5.0");
    assert_eq!(auth.view.team_rpm_limit, Some(-10));
    assert_eq!(auth.extra.user_max_budget.unwrap().to_string(), "-2.5");
}

#[test]
fn resolved_key_nonfinite_or_range_error() {
    let fields = object("{\"last_refreshed_at\": 1e999}");
    VerificationTokenView::construct(fields).unwrap_err();
}

#[test]
fn resolved_key_debug_redacts_credentials() {
    let auth =
        UserAPIKeyAuth::construct(AuthInput::Dict(object("{\"api_key\": \"sk-realsecret\"}")))
            .unwrap_or_else(|error| panic!("fixture should construct: {error}"));
    let hashed = credential::hash_token("sk-realsecret");
    let debug_text = format!("{auth:?}");
    assert!(!debug_text.contains("sk-realsecret"));
    assert!(!debug_text.contains(&hashed));
    assert_eq!(
        auth.extra
            .api_key
            .0
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(hashed.as_str())
    );
}
