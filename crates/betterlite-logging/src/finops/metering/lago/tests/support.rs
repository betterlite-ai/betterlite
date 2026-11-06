#![cfg(test)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::Deserialize;

use crate::finops::metering::lago::LagoLogger;
use crate::finops::metering::lago::settings::LagoSettings;
use crate::finops::metering::support::{FakeSender, variables};

pub(super) const API_KEY: &str = "lago-key-1234";
pub(super) const EVENT_CODE: &str = "litellm-event";
pub(super) const API_BASE: &str = "https://lago.test";

pub(super) fn environment_with(overrides: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut values = variables(&[
        ("LAGO_API_KEY", API_KEY),
        ("LAGO_API_BASE", API_BASE),
        ("LAGO_API_EVENT_CODE", EVENT_CODE),
    ]);
    values.extend(
        overrides
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
    );
    values
}

pub(super) fn settings_with(overrides: &[(&str, &str)]) -> LagoSettings {
    LagoSettings::from_environment(&environment_with(overrides)).unwrap()
}

pub(super) fn logger(sender: FakeSender) -> LagoLogger<FakeSender> {
    logger_with(sender, &[])
}

pub(super) fn logger_with(
    sender: FakeSender,
    overrides: &[(&str, &str)],
) -> LagoLogger<FakeSender> {
    let settings = LagoSettings::from_environment(&environment_with(overrides)).unwrap();
    LagoLogger { settings, sender }
}

#[derive(Debug, Deserialize)]
pub(super) struct LagoBody {
    pub(super) event: LagoBodyEvent,
}

#[derive(Debug, Deserialize)]
pub(super) struct LagoBodyEvent {
    pub(super) transaction_id: String,
    pub(super) external_subscription_id: String,
    pub(super) code: String,
    pub(super) properties: LagoBodyProperties,
}

#[derive(Debug, Deserialize, PartialEq)]
pub(super) struct LagoBodyProperties {
    pub(super) model: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub(super) response_cost: Option<Decimal>,
    #[serde(default)]
    pub(super) prompt_tokens: Option<i64>,
    #[serde(default)]
    pub(super) completion_tokens: Option<i64>,
    #[serde(default)]
    pub(super) total_tokens: Option<i64>,
}
