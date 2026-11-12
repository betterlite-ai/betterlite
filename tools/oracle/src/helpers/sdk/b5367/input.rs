use std::collections::BTreeMap;

use betterlite_redact::secret::MINIMUM_CUSTOM_KEY_LENGTH;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_setup::RequestMap;
use chrono::DateTime;
use chrono::FixedOffset;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use super::support::CredentialSpec;
use super::support::Decision;
use super::support::EntrySpec;
use super::support::OutcomeSpec;
use super::support::helper_error;

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    #[default]
    Setup,
    Credentials,
    RequestFlags,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(super) action: Action,
    #[serde(default)]
    pub(super) call_type: String,
    #[serde(default)]
    pub(super) is_async_call: Option<bool>,
    #[serde(default)]
    pub(super) is_pass_through: Option<bool>,
    #[serde(default)]
    pub(super) request: Value,
    #[serde(default)]
    pub(super) lists: super::support::Lists,
    #[serde(default)]
    pub(super) callbacks: Vec<EntrySpec>,
    #[serde(default)]
    pub(super) resolver: BTreeMap<String, OutcomeSpec>,
    #[serde(default)]
    pub(super) rules: Vec<Option<Decision>>,
    #[serde(default)]
    pub(super) provider_map: Vec<String>,
    #[serde(default)]
    pub(super) providers: Vec<String>,
    #[serde(default)]
    pub(super) custom_providers: Vec<String>,
    #[serde(default)]
    pub(super) credentials: Vec<CredentialSpec>,
}

pub(super) fn request_map(value: &Value) -> Result<RequestMap, Box<crate::helpers::HelperError>> {
    if value.is_null() {
        return Ok(IndexMap::new());
    }
    serde_json::from_value(value.clone())
        .map_err(|error| helper_error("InvalidInput", error.to_string()))
}

pub(super) fn redactor() -> Result<Redactor, Box<crate::helpers::HelperError>> {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| helper_error("Redaction", error.to_string()))
}

pub(super) fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
        .unwrap_or_else(|_error| DateTime::UNIX_EPOCH.fixed_offset())
}
