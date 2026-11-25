use std::fmt::Display;

use betterlite_config::sections::key_limit_policy::UpperboundKeyGenerateParams;
use betterlite_core::duration::seconds::duration_in_seconds;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

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

// fields run in the request model's declaration order, the order LiteLLM iterates the request in
pub fn enforce_upperbound(
    request: &KeyLimitRequest,
    params: Option<&UpperboundKeyGenerateParams>,
    fill: bool,
    time: &TimeContext<'_>,
) -> Result<UpperboundValues, KeyLimitError> {
    check_finite(&request.numeric_values())?;
    let mut max_budget = request.max_budget;
    let mut values = UpperboundValues {
        max_budget: None,
        max_parallel_requests: request.max_parallel_requests,
        tpm_limit: request.tpm_limit,
        rpm_limit: request.rpm_limit,
        duration: request.duration.clone(),
        budget_duration: request.budget_duration.clone(),
    };
    if let Some(ceilings) = params {
        let duration = ceilings.duration.as_deref();
        apply_duration("duration", &mut values.duration, duration, fill, time)?;
        apply_ceiling("max_budget", &mut max_budget, ceilings.max_budget, fill)?;
        let parallel = ceilings.max_parallel_requests;
        apply_ceiling(
            "max_parallel_requests",
            &mut values.max_parallel_requests,
            parallel,
            fill,
        )?;
        apply_ceiling("tpm_limit", &mut values.tpm_limit, ceilings.tpm_limit, fill)?;
        apply_ceiling("rpm_limit", &mut values.rpm_limit, ceilings.rpm_limit, fill)?;
        let budget_duration = ceilings.budget_duration.as_deref();
        apply_duration(
            "budget_duration",
            &mut values.budget_duration,
            budget_duration,
            fill,
            time,
        )?;
    }
    values.max_budget = max_budget.map(Amount);
    Ok(values)
}

// the GHSA-2rv4-xv66-fpjg guard runs before the missing-config return, for the six budget numerics
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

fn apply_ceiling<T>(
    field: &'static str,
    value: &mut Option<T>,
    ceiling: Option<T>,
    fill: bool,
) -> Result<(), Rejection>
where
    T: Copy + PartialOrd + Display,
{
    let Some(cap) = ceiling else {
        return Ok(());
    };
    match *value {
        Some(asked) if asked > cap => Err(over(field, asked, cap)),
        None if fill => {
            *value = Some(cap);
            Ok(())
        }
        Some(_) | None => Ok(()),
    }
}

// the configured ceiling parses through B199 as written; only the caller's "-1" means infinite
fn apply_duration(
    field: &'static str,
    value: &mut Option<String>,
    ceiling: Option<&str>,
    fill: bool,
    time: &TimeContext<'_>,
) -> Result<(), KeyLimitError> {
    let Some(cap_text) = ceiling else {
        return Ok(());
    };
    let Some(asked) = value.as_deref() else {
        if fill {
            *value = Some(cap_text.to_owned());
        }
        return Ok(());
    };
    let seconds = |text: &str| {
        duration_in_seconds(text, time.now, time.timezone, time.database)
            .map(|budget| budget.seconds)
            .map_err(|source| KeyLimitError::Duration { field, source })
    };
    let cap = seconds(cap_text)?;
    let exceeds = match asked {
        "-1" => true,
        finite => seconds(finite)? > cap,
    };
    if exceeds {
        return Err(KeyLimitError::from(over(field, asked, cap_text)));
    }
    Ok(())
}

fn over<T>(field: &'static str, user_value: T, max_value: T) -> Rejection
where
    T: Display,
{
    Rejection::Upperbound {
        field,
        user_value: user_value.to_string(),
        max_value: max_value.to_string(),
    }
}
