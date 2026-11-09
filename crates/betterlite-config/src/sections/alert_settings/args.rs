use rust_decimal::Decimal;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_integer, resolve_number};
use crate::sections::alert_settings::types::SlackAlertingArgs;

const ARGS_PATH: &str = "general_settings.alerting_args";
const MIN_BASELINE_DAYS: u32 = 1;
const MIN_USER_SPEND_CHECK_INTERVAL: u32 = 60;

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
    log_to_console: Option<bool>,
    #[serde(rename = "daily_spend_per_user_threshold")]
    daily_user_spend_threshold: Option<RawNumber>,
    #[serde(rename = "monthly_spend_per_user_threshold")]
    monthly_user_spend_threshold: Option<RawNumber>,
    spend_anomaly_multiplier: Option<RawNumber>,
    spend_anomaly_baseline_days: Option<RawNumber>,
    spend_anomaly_min_spend: Option<RawNumber>,
    user_spend_check_interval: Option<RawNumber>,
}

struct Timings {
    daily_report_frequency: u32,
    report_check_interval: u32,
    budget_alert_ttl: u32,
    outage_alert_ttl: u32,
    region_outage_alert_ttl: u32,
}

struct Outages {
    minor_outage_alert_threshold: u32,
    major_outage_alert_threshold: u32,
    max_outage_alert_list_size: u32,
    log_to_console: bool,
}

struct Spends {
    daily_user_spend_threshold: Option<Decimal>,
    monthly_user_spend_threshold: Option<Decimal>,
    spend_anomaly_multiplier: Decimal,
    spend_anomaly_baseline_days: u32,
    spend_anomaly_min_spend: Decimal,
    user_spend_check_interval: u32,
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
        let mut raw = self;
        let timings = raw.resolve_timings(lookup, environment_frequency)?;
        let outages = raw.resolve_outages(lookup)?;
        let spends = raw.resolve_spends(lookup)?;
        Ok(SlackAlertingArgs {
            daily_report_frequency: timings.daily_report_frequency,
            report_check_interval: timings.report_check_interval,
            budget_alert_ttl: timings.budget_alert_ttl,
            outage_alert_ttl: timings.outage_alert_ttl,
            region_outage_alert_ttl: timings.region_outage_alert_ttl,
            minor_outage_alert_threshold: outages.minor_outage_alert_threshold,
            major_outage_alert_threshold: outages.major_outage_alert_threshold,
            max_outage_alert_list_size: outages.max_outage_alert_list_size,
            log_to_console: outages.log_to_console,
            daily_user_spend_threshold: spends.daily_user_spend_threshold,
            monthly_user_spend_threshold: spends.monthly_user_spend_threshold,
            spend_anomaly_multiplier: spends.spend_anomaly_multiplier,
            spend_anomaly_baseline_days: spends.spend_anomaly_baseline_days,
            spend_anomaly_min_spend: spends.spend_anomaly_min_spend,
            user_spend_check_interval: spends.user_spend_check_interval,
        })
    }

    fn resolve_timings<L>(
        &mut self,
        lookup: &L,
        environment_frequency: u32,
    ) -> Result<Timings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let defaults = SlackAlertingArgs::default();
        Ok(Timings {
            daily_report_frequency: count_or(
                self.daily_report_frequency.take(),
                "daily_report_frequency",
                environment_frequency,
                lookup,
            )?,
            report_check_interval: count_or(
                self.report_check_interval.take(),
                "report_check_interval",
                defaults.report_check_interval,
                lookup,
            )?,
            budget_alert_ttl: count_or(
                self.budget_alert_ttl.take(),
                "budget_alert_ttl",
                defaults.budget_alert_ttl,
                lookup,
            )?,
            outage_alert_ttl: count_or(
                self.outage_alert_ttl.take(),
                "outage_alert_ttl",
                defaults.outage_alert_ttl,
                lookup,
            )?,
            region_outage_alert_ttl: count_or(
                self.region_outage_alert_ttl.take(),
                "region_outage_alert_ttl",
                defaults.region_outage_alert_ttl,
                lookup,
            )?,
        })
    }

    fn resolve_outages<L>(&mut self, lookup: &L) -> Result<Outages, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let defaults = SlackAlertingArgs::default();
        Ok(Outages {
            minor_outage_alert_threshold: count_or(
                self.minor_outage_alert_threshold.take(),
                "minor_outage_alert_threshold",
                defaults.minor_outage_alert_threshold,
                lookup,
            )?,
            major_outage_alert_threshold: count_or(
                self.major_outage_alert_threshold.take(),
                "major_outage_alert_threshold",
                defaults.major_outage_alert_threshold,
                lookup,
            )?,
            max_outage_alert_list_size: count_or(
                self.max_outage_alert_list_size.take(),
                "max_outage_alert_list_size",
                defaults.max_outage_alert_list_size,
                lookup,
            )?,
            log_to_console: self
                .log_to_console
                .take()
                .unwrap_or(defaults.log_to_console),
        })
    }

    fn resolve_spends<L>(&mut self, lookup: &L) -> Result<Spends, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let defaults = SlackAlertingArgs::default();
        Ok(Spends {
            daily_user_spend_threshold: optional_money(
                self.daily_user_spend_threshold.take(),
                "daily_spend_per_user_threshold",
                lookup,
            )?,
            monthly_user_spend_threshold: optional_money(
                self.monthly_user_spend_threshold.take(),
                "monthly_spend_per_user_threshold",
                lookup,
            )?,
            spend_anomaly_multiplier: required_money(
                self.spend_anomaly_multiplier.take(),
                "spend_anomaly_multiplier",
                defaults.spend_anomaly_multiplier,
                lookup,
            )?,
            spend_anomaly_baseline_days: bounded_count(
                self.spend_anomaly_baseline_days.take(),
                "spend_anomaly_baseline_days",
                defaults.spend_anomaly_baseline_days,
                MIN_BASELINE_DAYS,
                lookup,
            )?,
            spend_anomaly_min_spend: required_money(
                self.spend_anomaly_min_spend.take(),
                "spend_anomaly_min_spend",
                defaults.spend_anomaly_min_spend,
                lookup,
            )?,
            user_spend_check_interval: bounded_count(
                self.user_spend_check_interval.take(),
                "user_spend_check_interval",
                defaults.user_spend_check_interval,
                MIN_USER_SPEND_CHECK_INTERVAL,
                lookup,
            )?,
        })
    }
}

fn count_or<L>(
    value: Option<RawNumber>,
    field: &str,
    default: u32,
    lookup: &L,
) -> Result<u32, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("{ARGS_PATH}.{field}");
    let resolved = resolve_number(value, &path, lookup, parse_integer)?;
    Ok(resolved.unwrap_or(default))
}

fn optional_money<L>(
    value: Option<RawNumber>,
    field: &str,
    lookup: &L,
) -> Result<Option<Decimal>, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("{ARGS_PATH}.{field}");
    let resolved = resolve_number(value, &path, lookup, parse_money)?;
    match resolved {
        Some(amount) => Ok(Some(positive(amount, &path)?)),
        None => Ok(None),
    }
}

fn required_money<L>(
    value: Option<RawNumber>,
    field: &str,
    default: Decimal,
    lookup: &L,
) -> Result<Decimal, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("{ARGS_PATH}.{field}");
    let resolved = resolve_number(value, &path, lookup, parse_money)?.unwrap_or(default);
    positive(resolved, &path)
}

fn bounded_count<L>(
    value: Option<RawNumber>,
    field: &str,
    default: u32,
    minimum: u32,
    lookup: &L,
) -> Result<u32, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("{ARGS_PATH}.{field}");
    let resolved = count_or(value, field, default, lookup)?;
    if resolved < minimum {
        return Err(ConfigError::InvalidScalar { path, value: false });
    }
    Ok(resolved)
}

fn parse_money(text: &str, path: &str) -> Result<Decimal, ConfigError> {
    text.parse::<Decimal>()
        .map_err(|_source| ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        })
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
