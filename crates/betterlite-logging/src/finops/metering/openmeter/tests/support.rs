#![cfg(test)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::Deserialize;

use crate::finops::metering::openmeter::OpenMeterLogger;
use crate::finops::metering::openmeter::settings::OpenMeterSettings;
use crate::finops::metering::support::{FakeSender, FixedClock, variables};

pub(super) const API_KEY: &str = "openmeter-key-1234";
pub(super) const ENDPOINT: &str = "https://openmeter.test";

pub(super) fn environment_with(overrides: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut values = variables(&[
        ("OPENMETER_API_KEY", API_KEY),
        ("OPENMETER_API_ENDPOINT", ENDPOINT),
    ]);
    values.extend(
        overrides
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
    );
    values
}

pub(super) fn settings_with(overrides: &[(&str, &str)]) -> OpenMeterSettings {
    OpenMeterSettings::from_environment(&environment_with(overrides)).unwrap()
}

pub(super) fn logger(sender: FakeSender) -> OpenMeterLogger<FakeSender, FixedClock> {
    logger_with(sender, &[])
}

pub(super) fn logger_with(
    sender: FakeSender,
    overrides: &[(&str, &str)],
) -> OpenMeterLogger<FakeSender, FixedClock> {
    let settings = OpenMeterSettings::from_environment(&environment_with(overrides)).unwrap();
    OpenMeterLogger {
        settings,
        sender,
        clock: FixedClock,
    }
}

#[derive(Debug, Deserialize, PartialEq)]
pub(super) struct CloudEventBody {
    pub(super) specversion: String,
    #[serde(rename = "type")]
    pub(super) event_type: String,
    pub(super) id: Option<String>,
    pub(super) time: String,
    pub(super) subject: String,
    pub(super) source: String,
    pub(super) data: CloudEventDataBody,
}

#[derive(Debug, Deserialize, PartialEq)]
pub(super) struct CloudEventDataBody {
    pub(super) model: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub(super) cost: Option<Decimal>,
    #[serde(default)]
    pub(super) prompt_tokens: Option<i64>,
    #[serde(default)]
    pub(super) completion_tokens: Option<i64>,
    #[serde(default)]
    pub(super) total_tokens: Option<i64>,
}
