use betterlite_config::sections::settings::JsonValue;

use super::super::error::ConfigFieldError;
use super::is_integer;

#[derive(Clone, Copy)]
pub(crate) enum ArgKind {
    Int,
    IntAtLeast(u32),
    Bool,
    Float,
    FloatNullable,
}

const ALERTING_ARGS: [(&str, ArgKind); 15] = [
    ("daily_report_frequency", ArgKind::Int),
    ("report_check_interval", ArgKind::Int),
    ("budget_alert_ttl", ArgKind::Int),
    ("outage_alert_ttl", ArgKind::Int),
    ("region_outage_alert_ttl", ArgKind::Int),
    ("minor_outage_alert_threshold", ArgKind::Int),
    ("major_outage_alert_threshold", ArgKind::Int),
    ("max_outage_alert_list_size", ArgKind::Int),
    ("log_to_console", ArgKind::Bool),
    ("daily_spend_per_user_threshold", ArgKind::FloatNullable),
    ("monthly_spend_per_user_threshold", ArgKind::FloatNullable),
    ("spend_anomaly_multiplier", ArgKind::Float),
    ("spend_anomaly_baseline_days", ArgKind::IntAtLeast(1)),
    ("spend_anomaly_min_spend", ArgKind::Float),
    ("user_spend_check_interval", ArgKind::IntAtLeast(60)),
];

fn integer_message(value: &JsonValue) -> String {
    let fractional = value
        .as_f64()
        .is_some_and(|numeric| numeric.fract() != 0.0_f64);
    if fractional {
        return "Input should be a valid integer, got a number with a fractional part".to_owned();
    }
    "Input should be a valid integer".to_owned()
}

fn arg_error(name: &str, kind: ArgKind, value: &JsonValue) -> Option<String> {
    match kind {
        ArgKind::Int | ArgKind::IntAtLeast(_) if !is_integer(value) => {
            Some(format!("{name}: {}", integer_message(value)))
        }
        ArgKind::IntAtLeast(minimum)
            if value
                .as_f64()
                .is_none_or(|numeric| numeric < f64::from(minimum)) =>
        {
            Some(format!(
                "{name}: Input should be greater than or equal to {minimum}"
            ))
        }
        ArgKind::Bool if !value.is_boolean() => {
            Some(format!("{name}: Input should be a valid boolean"))
        }
        ArgKind::Float | ArgKind::FloatNullable if !value.is_number() => {
            Some(format!("{name}: Input should be a valid number"))
        }
        ArgKind::Float | ArgKind::FloatNullable
            if value.as_f64().is_none_or(|numeric| numeric <= 0.0_f64) =>
        {
            Some(format!("{name}: Input should be greater than 0"))
        }
        ArgKind::Int
        | ArgKind::IntAtLeast(_)
        | ArgKind::Bool
        | ArgKind::Float
        | ArgKind::FloatNullable => None,
    }
}

// SlackAlertingArgs.model_validate(value): the row stores the dict as sent, so this only refuses what pydantic refuses
pub(crate) fn validate_alerting_args(value: &JsonValue) -> Result<(), ConfigFieldError> {
    let Some(entries) = value.as_object() else {
        return Err(alerting_error(
            ": Input should be a valid dictionary or instance of SlackAlertingArgs",
        ));
    };
    let mut errors: Vec<String> = Vec::new();
    for (name, kind) in ALERTING_ARGS {
        let Some(entry) = entries.get(name) else {
            continue;
        };
        if matches!(kind, ArgKind::FloatNullable) && entry.is_null() {
            continue;
        }
        if let Some(error) = arg_error(name, kind, entry) {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        return Ok(());
    }
    Err(alerting_error(&errors.join("; ")))
}

fn alerting_error(errors: &str) -> ConfigFieldError {
    ConfigFieldError::BadRequest {
        detail: format!("Invalid alerting_args: {errors}"),
    }
}
