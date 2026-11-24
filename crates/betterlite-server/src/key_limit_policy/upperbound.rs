use std::fmt::Display;

use betterlite_config::sections::key_limit_policy::UpperboundKeyGenerateParams;
use betterlite_core::duration::seconds::duration_in_seconds;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use crate::key_fields::values::Amount;
use crate::key_limit_policy::error::{KeyLimitError, Rejection};
use crate::key_limit_policy::request::{KeyLimitRequest, NumericValue};
use crate::management_guards::amounts::{Amount as GuardAmount, describe_amount};

// the request's upperbound-managed fields after enforcement, which the caller writes back
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UpperboundValues {
    pub max_budget: Option<Amount>,
    pub max_parallel_requests: Option<i64>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub duration: Option<String>,
    pub budget_duration: Option<String>,
}

// B199's duration parser needs the caller's clock, timezone and zone database
pub struct TimeContext<'time> {
    pub now: DateTime<Utc>,
    pub timezone: Option<&'time str>,
    pub database: &'time TimeZoneDatabase,
}

pub fn enforce_upperbound(
    request: &KeyLimitRequest<'_>,
    params: Option<&UpperboundKeyGenerateParams>,
    fill_defaults: bool,
    time: &TimeContext<'_>,
) -> Result<UpperboundValues, KeyLimitError> {
    check_finite(&request.numeric_values())?;
    let mut values = UpperboundValues::of(request);
    let Some(params) = params else {
        return Ok(values);
    };
    apply_duration(
        "duration",
        &mut values.duration,
        params.duration.as_deref(),
        fill_defaults,
        time,
    )?;
    apply_limits(&mut values, params, fill_defaults)?;
    apply_duration(
        "budget_duration",
        &mut values.budget_duration,
        params.budget_duration.as_deref(),
        fill_defaults,
        time,
    )?;
    Ok(values)
}

// the four numeric ceilings, in the request field declaration order LiteLLM iterates
fn apply_limits(
    values: &mut UpperboundValues,
    params: &UpperboundKeyGenerateParams,
    fill_defaults: bool,
) -> Result<(), Rejection> {
    apply_money(
        "max_budget",
        &mut values.max_budget,
        params.max_budget,
        fill_defaults,
    )?;
    apply_count(
        "max_parallel_requests",
        &mut values.max_parallel_requests,
        params.max_parallel_requests,
        fill_defaults,
    )?;
    apply_count(
        "tpm_limit",
        &mut values.tpm_limit,
        params.tpm_limit,
        fill_defaults,
    )?;
    apply_count(
        "rpm_limit",
        &mut values.rpm_limit,
        params.rpm_limit,
        fill_defaults,
    )
}

// The GHSA-2rv4-xv66-fpjg guard: it runs before the missing-config return, and every model field it
// names is declared float or int, so only the two money fields can actually carry a non-finite value.
pub fn check_finite(values: &[NumericValue]) -> Result<(), Rejection> {
    for item in values {
        if let Some(amount) = item.value
            && !matches!(amount, GuardAmount::Finite(_))
        {
            return Err(Rejection::NonFinite {
                field: item.field,
                received: describe_amount(Some(amount)),
            });
        }
    }
    Ok(())
}

impl UpperboundValues {
    fn of(request: &KeyLimitRequest<'_>) -> Self {
        Self {
            max_budget: request.max_budget().map(Amount),
            max_parallel_requests: request.max_parallel_requests(),
            tpm_limit: request.tpm_limit(),
            rpm_limit: request.rpm_limit(),
            duration: request.duration().map(str::to_owned),
            budget_duration: request.budget_duration().map(str::to_owned),
        }
    }
}

fn apply_money(
    field: &'static str,
    value: &mut Option<Amount>,
    ceiling: Option<Decimal>,
    fill_defaults: bool,
) -> Result<(), Rejection> {
    let Some(cap) = ceiling else {
        return Ok(());
    };
    match *value {
        Some(asked) if asked.0 > cap => Err(over(field, asked.0, cap)),
        Some(_) => Ok(()),
        None => {
            if fill_defaults {
                *value = Some(Amount(cap));
            }
            Ok(())
        }
    }
}

fn apply_count(
    field: &'static str,
    value: &mut Option<i64>,
    ceiling: Option<i64>,
    fill_defaults: bool,
) -> Result<(), Rejection> {
    let Some(cap) = ceiling else {
        return Ok(());
    };
    match *value {
        Some(asked) if asked > cap => Err(over(field, asked, cap)),
        Some(_) => Ok(()),
        None => {
            if fill_defaults {
                *value = Some(cap);
            }
            Ok(())
        }
    }
}

// the configured ceiling parses through B199 as written; only the caller's "-1" means infinite
fn apply_duration(
    field: &'static str,
    value: &mut Option<String>,
    ceiling: Option<&str>,
    fill_defaults: bool,
    time: &TimeContext<'_>,
) -> Result<(), KeyLimitError> {
    let Some(cap_text) = ceiling else {
        return Ok(());
    };
    let Some(asked) = value.clone() else {
        if fill_defaults {
            *value = Some(cap_text.to_owned());
        }
        return Ok(());
    };
    let cap = duration_in_seconds(cap_text, time.now, time.timezone, time.database)
        .map(|budget| budget.seconds)
        .map_err(|source| KeyLimitError::Duration { field, source })?;
    let asked_seconds = if asked == "-1" {
        None
    } else {
        Some(
            duration_in_seconds(&asked, time.now, time.timezone, time.database)
                .map(|budget| budget.seconds)
                .map_err(|source| KeyLimitError::Duration { field, source })?,
        )
    };
    if exceeds(asked_seconds, cap) {
        return Err(KeyLimitError::from(over(field, &asked, cap_text)));
    }
    Ok(())
}

fn exceeds(asked: Option<i64>, cap: i64) -> bool {
    match asked {
        Some(seconds) => seconds > cap,
        None => true,
    }
}

fn over<U, C>(field: &'static str, user_value: U, max_value: C) -> Rejection
where
    U: Display,
    C: Display,
{
    Rejection::Upperbound {
        field,
        user_value: user_value.to_string(),
        max_value: max_value.to_string(),
    }
}
