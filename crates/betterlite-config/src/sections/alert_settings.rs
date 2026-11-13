use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

use self::args::RawAlertingArgs;
use self::digest::RawAlertTypeConfig;
use self::types::{
    AlertType, AlertTypeConfig, DEFAULT_ALERT_TYPES, DEFAULT_DAILY_REPORT_FREQUENCY,
    SlackAlertingArgs,
};
use self::webhooks::{RawWebhookUrl, WebhookUrl, resolve_alerting_webhooks};

pub mod args;
pub mod digest;
pub mod types;
pub mod webhooks;

mod flags;
mod tests;

const ALERTING_THRESHOLD_PATH: &str = "general_settings.alerting_threshold";
const SLACK_DAILY_REPORT_FREQUENCY: &str = "SLACK_DAILY_REPORT_FREQUENCY";
const DEFAULT_ALERTING_THRESHOLD: f64 = 300.0;
const ENABLED_ALERTING_THRESHOLD: f64 = 600.0;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    alerting: Option<Vec<String>>,
    #[serde(default)]
    alerting_threshold: RawThreshold,
    alert_types: Option<Vec<AlertType>>,
    alert_to_webhook_url: Option<BTreeMap<AlertType, RawWebhookUrl>>,
    alerting_args: Option<RawAlertingArgs>,
    alert_type_config: Option<BTreeMap<AlertType, RawAlertTypeConfig>>,
}

// Missing comes from the field default, so an absent key differs from an explicit null
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum RawThreshold {
    Number(RawNumber),
    Null(()),
    #[default]
    Missing,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub alerting: Vec<String>,
    pub alerting_threshold_seconds: f64,
    pub alert_types: Vec<AlertType>,
    pub alert_to_webhook_url: BTreeMap<AlertType, WebhookUrl>,
    pub alerting_args: SlackAlertingArgs,
    pub alert_type_config: BTreeMap<AlertType, AlertTypeConfig>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            alerting: Vec::new(),
            alerting_threshold_seconds: DEFAULT_ALERTING_THRESHOLD,
            alert_types: DEFAULT_ALERT_TYPES.to_vec(),
            alert_to_webhook_url: BTreeMap::new(),
            alerting_args: SlackAlertingArgs::default(),
            alert_type_config: BTreeMap::new(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        let environment_frequency = daily_report_frequency(lookup)?;
        let alert_type_config =
            resolve_alert_type_config(general.alert_type_config.unwrap_or_default(), lookup)?;
        let alerting_threshold = alerting_threshold(
            general.alerting.as_deref(),
            general.alerting_threshold,
            lookup,
        )?;
        Ok(Settings {
            alerting: general.alerting.unwrap_or_default(),
            alerting_threshold_seconds: alerting_threshold,
            alert_types: general
                .alert_types
                .unwrap_or_else(|| DEFAULT_ALERT_TYPES.to_vec()),
            alert_to_webhook_url: resolve_alerting_webhooks(
                general.alert_to_webhook_url.unwrap_or_default(),
                lookup,
            )?,
            alerting_args: general
                .alerting_args
                .unwrap_or_default()
                .resolve(lookup, environment_frequency)?,
            alert_type_config,
        })
    }
}

// LiteLLM reads the threshold as 600 only on its alerting path, which it skips when alerting is unset
fn alerting_threshold<L>(
    alerting: Option<&[String]>,
    threshold: RawThreshold,
    lookup: &L,
) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    if alerting.is_none() {
        return Ok(DEFAULT_ALERTING_THRESHOLD);
    }
    match threshold {
        RawThreshold::Missing => Ok(ENABLED_ALERTING_THRESHOLD),
        RawThreshold::Null(()) => Ok(DEFAULT_ALERTING_THRESHOLD),
        RawThreshold::Number(number) => {
            let parsed =
                resolve_number(Some(number), ALERTING_THRESHOLD_PATH, lookup, parse_float)?;
            Ok(parsed.unwrap_or(DEFAULT_ALERTING_THRESHOLD))
        }
    }
}

fn resolve_alert_type_config<L>(
    raw: BTreeMap<AlertType, RawAlertTypeConfig>,
    lookup: &L,
) -> Result<BTreeMap<AlertType, AlertTypeConfig>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = BTreeMap::new();
    for (alert_type, config) in raw {
        resolved.insert(alert_type, config.resolve(alert_type, lookup)?);
    }
    Ok(resolved)
}

fn daily_report_frequency<L>(lookup: &L) -> Result<u32, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(SLACK_DAILY_REPORT_FREQUENCY) else {
        return Ok(DEFAULT_DAILY_REPORT_FREQUENCY);
    };
    let frequency = betterlite_env::number::<u32>(SLACK_DAILY_REPORT_FREQUENCY, &text)?;
    Ok(frequency.unwrap_or(DEFAULT_DAILY_REPORT_FREQUENCY))
}
