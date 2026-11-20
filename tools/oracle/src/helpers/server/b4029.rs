use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::alerting::user_spend_alerts::{
    UserSpendAlertEvent, UserSpendAlertKind, UserSpendChecks, UserSpendRow, evaluate_user_spend,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// HOURS_IN_A_DAY of litellm/constants.py, which the oracle runs without an override
const HOURS_IN_A_DAY: u32 = 24;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    row: RowInput,
    settings: SettingsInput,
    today: String,
    thresholds_enabled: bool,
    anomalies_enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RowInput {
    user_id: String,
    daily_spend: f64,
    monthly_spend: f64,
    baseline_spend: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsInput {
    daily_spend_per_user_threshold: Option<f64>,
    monthly_spend_per_user_threshold: Option<f64>,
    spend_anomaly_multiplier: f64,
    spend_anomaly_baseline_days: u32,
    spend_anomaly_min_spend: f64,
}

#[derive(Serialize)]
struct EventView<'event> {
    kind: UserSpendAlertKind,
    alert_type: AlertType,
    message: &'event str,
    cache_key: &'event str,
    cache_ttl: u64,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let args = alerting_args(&input.settings)?;
        let row = UserSpendRow {
            user_id: input.row.user_id.clone(),
            daily_spend: input.row.daily_spend,
            monthly_spend: input.row.monthly_spend,
            baseline_spend: input.row.baseline_spend,
        };
        let today = NaiveDate::parse_from_str(&input.today, "%Y-%m-%d").or_value_error()?;
        let checks = UserSpendChecks {
            thresholds: input.thresholds_enabled,
            anomalies: input.anomalies_enabled,
        };
        let events = evaluate_user_spend(&row, &args, today, checks, HOURS_IN_A_DAY);
        let views: Vec<EventView<'_>> = events
            .iter()
            .map(|event: &UserSpendAlertEvent| EventView {
                kind: event.kind,
                alert_type: event.alert_type,
                message: &event.message,
                cache_key: &event.cache_key,
                cache_ttl: event.cache_ttl,
            })
            .collect();
        Ok(serde_json::to_value(views)?)
    }
}

fn alerting_args(settings: &SettingsInput) -> Result<SlackAlertingArgs, Box<HelperError>> {
    Ok(SlackAlertingArgs {
        daily_report_frequency: 43_200,
        report_check_interval: 300,
        budget_alert_ttl: 86_400,
        outage_alert_ttl: 60,
        region_outage_alert_ttl: 60,
        minor_outage_alert_threshold: 5,
        major_outage_alert_threshold: 10,
        max_outage_alert_list_size: 10,
        log_to_console: false,
        daily_user_spend_threshold: settings
            .daily_spend_per_user_threshold
            .map(money)
            .transpose()?,
        monthly_user_spend_threshold: settings
            .monthly_spend_per_user_threshold
            .map(money)
            .transpose()?,
        spend_anomaly_multiplier: money(settings.spend_anomaly_multiplier)?,
        spend_anomaly_baseline_days: settings.spend_anomaly_baseline_days,
        spend_anomaly_min_spend: money(settings.spend_anomaly_min_spend)?,
        user_spend_check_interval: 3_600,
    })
}

// pydantic parses these settings as Python floats, so the case's f64 is the source value
fn money(value: f64) -> Result<Decimal, Box<HelperError>> {
    Decimal::from_f64_retain(value)
        .ok_or("a spend setting must be a finite number")
        .or_value_error()
}

mod tests;
