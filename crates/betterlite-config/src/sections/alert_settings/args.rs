use rust_decimal::Decimal;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_integer, parse_money, resolve_number};
use crate::sections::alert_settings::flags::{RawFlag, resolve_flag};
use crate::sections::alert_settings::types::{
    ANOMALY_BASELINE_DAYS, ANOMALY_MIN_SPEND, ANOMALY_MULTIPLIER, BUDGET_ALERT_TTL, CountField,
    DAILY_SPEND_THRESHOLD, MAJOR_OUTAGE_THRESHOLD, MAX_OUTAGE_LIST_SIZE, MINOR_OUTAGE_THRESHOLD,
    MONTHLY_SPEND_THRESHOLD, OUTAGE_ALERT_TTL, OptionalMoneyField, REGION_OUTAGE_TTL,
    REPORT_CHECK_INTERVAL, RequiredMoneyField, SPEND_CHECK_INTERVAL, SlackAlertingArgs,
};

const ARGS_PATH: &str = "general_settings.alerting_args";

#[derive(Debug, Default, Deserialize)]
pub struct RawAlertingArgs {
    daily_report_frequency: Option<RawNumber>,
    report_check_interval: Option<RawNumber>,
    budget_alert_ttl: Option<RawNumber>,
    outage_alert_ttl: Option<RawNumber>,
    region_outage_alert_ttl: Option<RawNumber>,
    minor_outage_alert_threshold: Option<RawNumber>,
    major_outage_alert_threshold: Option<RawNumber>,
    max_outage_alert_list_size: Option<RawNumber>,
    log_to_console: Option<RawFlag>,
    #[serde(rename = "daily_spend_per_user_threshold")]
    daily_spend: Option<RawNumber>,
    #[serde(rename = "monthly_spend_per_user_threshold")]
    monthly_spend: Option<RawNumber>,
    #[serde(rename = "spend_anomaly_multiplier")]
    anomaly_multiplier: Option<RawNumber>,
    #[serde(rename = "spend_anomaly_baseline_days")]
    anomaly_baseline_days: Option<RawNumber>,
    #[serde(rename = "spend_anomaly_min_spend")]
    anomaly_min_spend: Option<RawNumber>,
    #[serde(rename = "user_spend_check_interval")]
    spend_check_interval: Option<RawNumber>,
}

struct Resolver<'resolve, L> {
    lookup: &'resolve L,
}

impl<L: EnvironmentLookup> Resolver<'_, L> {
    fn count(&self, value: Option<RawNumber>, field: CountField) -> Result<u32, ConfigError> {
        let path = format!("{ARGS_PATH}.{}", field.name);
        let resolved =
            resolve_number(value, &path, self.lookup, parse_integer)?.unwrap_or(field.default);
        if resolved < field.minimum {
            return Err(ConfigError::InvalidScalar { path, value: false });
        }
        Ok(resolved)
    }

    fn optional_money(
        &self,
        value: Option<RawNumber>,
        field: OptionalMoneyField,
    ) -> Result<Option<Decimal>, ConfigError> {
        let path = format!("{ARGS_PATH}.{}", field.name);
        resolve_number(value, &path, self.lookup, parse_money)?
            .map_or(Ok(None), |amount| positive(amount, &path).map(Some))
    }

    fn required_money(
        &self,
        value: Option<RawNumber>,
        field: RequiredMoneyField,
    ) -> Result<Decimal, ConfigError> {
        let resolved = self.optional_money(value, OptionalMoneyField { name: field.name })?;
        if let Some(amount) = resolved {
            return Ok(amount);
        }
        Ok(field.default)
    }

    fn flag(&self, value: Option<RawFlag>, field: &str) -> Result<bool, ConfigError> {
        value.map_or(Ok(false), |raw| {
            resolve_flag(raw, &format!("{ARGS_PATH}.{field}"), self.lookup)
        })
    }
}

impl RawAlertingArgs {
    pub fn resolve<L>(
        self,
        lookup: &L,
        environment_frequency: u32,
    ) -> Result<SlackAlertingArgs, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let values = Resolver { lookup };
        let mut raw = self;
        let frequency = CountField {
            name: "daily_report_frequency",
            default: environment_frequency,
            minimum: 0,
        };
        Ok(SlackAlertingArgs {
            daily_report_frequency: values.count(raw.daily_report_frequency.take(), frequency)?,
            report_check_interval: values
                .count(raw.report_check_interval.take(), REPORT_CHECK_INTERVAL)?,
            budget_alert_ttl: values.count(raw.budget_alert_ttl.take(), BUDGET_ALERT_TTL)?,
            outage_alert_ttl: values.count(raw.outage_alert_ttl.take(), OUTAGE_ALERT_TTL)?,
            region_outage_alert_ttl: values
                .count(raw.region_outage_alert_ttl.take(), REGION_OUTAGE_TTL)?,
            minor_outage_alert_threshold: values.count(
                raw.minor_outage_alert_threshold.take(),
                MINOR_OUTAGE_THRESHOLD,
            )?,
            major_outage_alert_threshold: values.count(
                raw.major_outage_alert_threshold.take(),
                MAJOR_OUTAGE_THRESHOLD,
            )?,
            max_outage_alert_list_size: values
                .count(raw.max_outage_alert_list_size.take(), MAX_OUTAGE_LIST_SIZE)?,
            log_to_console: values.flag(raw.log_to_console.take(), "log_to_console")?,
            daily_user_spend_threshold: values
                .optional_money(raw.daily_spend.take(), DAILY_SPEND_THRESHOLD)?,
            monthly_user_spend_threshold: values
                .optional_money(raw.monthly_spend.take(), MONTHLY_SPEND_THRESHOLD)?,
            spend_anomaly_multiplier: values
                .required_money(raw.anomaly_multiplier.take(), ANOMALY_MULTIPLIER)?,
            spend_anomaly_baseline_days: values
                .count(raw.anomaly_baseline_days.take(), ANOMALY_BASELINE_DAYS)?,
            spend_anomaly_min_spend: values
                .required_money(raw.anomaly_min_spend.take(), ANOMALY_MIN_SPEND)?,
            user_spend_check_interval: values
                .count(raw.spend_check_interval.take(), SPEND_CHECK_INTERVAL)?,
        })
    }
}

fn positive(value: Decimal, path: &str) -> Result<Decimal, ConfigError> {
    if value <= Decimal::ZERO {
        return Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        });
    }
    Ok(value)
}
