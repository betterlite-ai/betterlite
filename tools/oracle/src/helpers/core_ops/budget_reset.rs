use betterlite_config::sections::budget_reset::{
    BudgetResetSettings, BudgetResetSettingsError, BudgetResetTimeError, get_budget_reset_settings,
    parse_budget_reset_time,
};
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::budget_reset::{
    ResetClocks, compute_budget_reset_at, is_persistable_budget_duration,
};
use betterlite_core::duration::error::{DurationError, DurationWarning};
use betterlite_core::duration::seconds::duration_in_seconds;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    #[serde(default)]
    timezone: JsonValue,
    #[serde(rename = "budget_reset_time", default)]
    reset_time_of_day: JsonValue,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    reset_now: String,
    calendar_now: String,
    #[serde(default)]
    calendar_timezone: Option<String>,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    ParseBudgetResetTime {
        value: JsonValue,
    },
    GetBudgetResetSettings {
        #[serde(default)]
        settings: RawSettings,
    },
    ComputeBudgetResetAt {
        #[serde(default)]
        settings: RawSettings,
        duration: String,
    },
    GetBudgetResetTime {
        #[serde(default)]
        settings: RawSettings,
        duration: String,
    },
    IsPersistable {
        #[serde(default)]
        settings: RawSettings,
        duration: String,
    },
    BudgetDurationError {
        #[serde(default)]
        settings: RawSettings,
        #[serde(default)]
        duration: Option<String>,
    },
}

#[derive(Debug, Serialize)]
struct Outcome {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
    warned: bool,
}

enum Answer {
    Value(JsonValue, bool),
    Failed(&'static str),
}

struct RunContext<'run> {
    clocks: ResetClocks<'run>,
    database: &'run TimeZoneDatabase,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let database = TimeZoneDatabase::from_env();
        let context = RunContext {
            clocks: ResetClocks {
                reset_now: moment(&input.reset_now)?,
                calendar_now: moment(&input.calendar_now)?,
                calendar_timezone: input.calendar_timezone.as_deref(),
            },
            database: &database,
        };
        let outcomes: Vec<Outcome> = input
            .calls
            .iter()
            .map(|call| run_call(call, &context))
            .collect();
        serde_json::to_value(outcomes).map_err(failure)
    }
}

fn run_call(call: &Call, context: &RunContext<'_>) -> Outcome {
    let answer = match call {
        Call::ParseBudgetResetTime { value } => parse_time(value),
        Call::GetBudgetResetSettings { settings } => current_settings(settings),
        Call::ComputeBudgetResetAt { settings, duration }
        | Call::GetBudgetResetTime { settings, duration } => {
            computed_at(settings, duration, context)
        }
        Call::IsPersistable { settings, duration } => Answer::Value(
            JsonValue::Bool(persists(settings, duration, context)),
            false,
        ),
        Call::BudgetDurationError { settings, duration } => Answer::Value(
            persistent_diagnostic(settings, duration.as_deref(), context),
            false,
        ),
    };
    match answer {
        Answer::Value(value, warned) => Outcome {
            ok: true,
            value: Some(value),
            error: None,
            warned,
        },
        Answer::Failed(category) => Outcome {
            ok: false,
            value: None,
            error: Some(category),
            warned: false,
        },
    }
}

fn parse_time(value: &JsonValue) -> Answer {
    match parse_budget_reset_time(value) {
        Ok(time) => Answer::Value(
            JsonValue::String(time.format("%H:%M:%S").to_string()),
            false,
        ),
        Err(error) => Answer::Failed(time_error_category(&error)),
    }
}

fn current_settings(settings: &RawSettings) -> Answer {
    match get_budget_reset_settings(&settings.timezone, &settings.reset_time_of_day) {
        Ok(resolved) => Answer::Value(settings_json(&resolved), false),
        Err(error) => Answer::Failed(settings_error_category(&error)),
    }
}

fn computed_at(settings: &RawSettings, duration: &str, context: &RunContext<'_>) -> Answer {
    let current = match get_budget_reset_settings(&settings.timezone, &settings.reset_time_of_day) {
        Ok(resolved) => resolved,
        Err(error) => return Answer::Failed(settings_error_category(&error)),
    };
    match compute_budget_reset_at(
        duration,
        &current,
        context.clocks.reset_now,
        context.database,
    ) {
        Ok(reset) => Answer::Value(
            JsonValue::from(reset.instant.timestamp_micros()),
            warned(&reset.warnings),
        ),
        Err(error) => Answer::Failed(duration_category(&error)),
    }
}

fn persists(settings: &RawSettings, duration: &str, context: &RunContext<'_>) -> bool {
    let Ok(budget) = duration_in_seconds(
        duration,
        context.clocks.calendar_now,
        context.clocks.calendar_timezone,
        context.database,
    ) else {
        return false;
    };
    if budget.seconds <= 0 {
        return false;
    }
    let Ok(current) = get_budget_reset_settings(&settings.timezone, &settings.reset_time_of_day)
    else {
        return false;
    };
    is_persistable_budget_duration(duration, &current, context.clocks, context.database)
        .unwrap_or(false)
}

fn persistent_diagnostic(
    settings: &RawSettings,
    duration: Option<&str>,
    context: &RunContext<'_>,
) -> JsonValue {
    let Some(budget_duration) = duration else {
        return JsonValue::Null;
    };
    if persists(settings, budget_duration, context) {
        return JsonValue::Null;
    }
    JsonValue::String("invalid".to_owned())
}

fn moment(text: &str) -> Result<DateTime<Utc>, Box<crate::helpers::HelperError>> {
    DateTime::parse_from_rfc3339(text)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(failure)
}

fn settings_json(settings: &BudgetResetSettings) -> JsonValue {
    json!({
        "timezone": settings.timezone.as_str(),
        "reset_time_of_day": settings.reset_time_of_day.format("%H:%M:%S").to_string(),
    })
}

fn warned(warnings: &[DurationWarning]) -> bool {
    warnings
        .iter()
        .any(|warning| matches!(warning, DurationWarning::UnrecognizedResetDuration { .. }))
}

const fn duration_category(error: &DurationError) -> &'static str {
    match error {
        DurationError::InvalidFormat { .. }
        | DurationError::UnsupportedUnit { .. }
        | DurationError::UnsupportedMonthInterval { .. } => "value_error",
        DurationError::QuantityOverflow { .. }
        | DurationError::SecondsOverflow { .. }
        | DurationError::CalendarOverflow
        | DurationError::CalendarField { .. }
        | DurationError::Calendar { .. }
        | DurationError::Timestamp { .. }
        | DurationError::TimestampOutOfRange => "overflow_error",
    }
}

const fn time_error_category(error: &BudgetResetTimeError) -> &'static str {
    match error {
        BudgetResetTimeError::NotText | BudgetResetTimeError::Clock(_) => "value_error",
    }
}

const fn settings_error_category(error: &BudgetResetSettingsError) -> &'static str {
    match error {
        BudgetResetSettingsError::TimezoneNotText | BudgetResetSettingsError::ResetTime(_) => {
            "value_error"
        }
    }
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "BudgetResetInput".to_owned(),
        message: error.to_string(),
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}
