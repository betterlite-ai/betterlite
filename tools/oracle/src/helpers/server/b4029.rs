use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};
use betterlite_oracle_helper_api::error::OrHelperError as _;
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
    daily_spend: Decimal,
    monthly_spend: Decimal,
    baseline_spend: Decimal,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsInput {
    daily_threshold: Option<Decimal>,
    monthly_threshold: Option<Decimal>,
    spend_anomaly_multiplier: Decimal,
    spend_anomaly_baseline_days: u32,
    spend_anomaly_min_spend: Decimal,
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
        let args = alerting_args(&input.settings);
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

const fn alerting_args(settings: &SettingsInput) -> SlackAlertingArgs {
    SlackAlertingArgs {
        daily_report_frequency: 43_200,
        report_check_interval: 300,
        budget_alert_ttl: 86_400,
        outage_alert_ttl: 60,
        region_outage_alert_ttl: 60,
        minor_outage_alert_threshold: 5,
        major_outage_alert_threshold: 10,
        max_outage_alert_list_size: 10,
        log_to_console: false,
        daily_user_spend_threshold: settings.daily_threshold,
        monthly_user_spend_threshold: settings.monthly_threshold,
        spend_anomaly_multiplier: settings.spend_anomaly_multiplier,
        spend_anomaly_baseline_days: settings.spend_anomaly_baseline_days,
        spend_anomaly_min_spend: settings.spend_anomaly_min_spend,
        user_spend_check_interval: 3_600,
    }
}

mod tests;
