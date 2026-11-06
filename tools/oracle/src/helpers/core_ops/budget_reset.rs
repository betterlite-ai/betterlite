use betterlite_config::sections::budget_reset::{
    BudgetResetSettings, BudgetResetSettingsError, BudgetResetTimeError, get_budget_reset_settings,
    parse_budget_reset_time,
};
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::budget_reset::{
    BudgetResetError, RawBudgetResetSettings, ResetClocks, budget_duration_error,
    compute_budget_reset_at, get_budget_reset_time, is_persistable_budget_duration,
};
use betterlite_core::duration::error::{DurationError, DurationWarning};
use betterlite_core::duration::reset::Reset;
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

impl RawSettings {
    const fn borrowed(&self) -> RawBudgetResetSettings<'_> {
        RawBudgetResetSettings {
            timezone: &self.timezone,
            reset_time_of_day: &self.reset_time_of_day,
        }
    }
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

type Answer = Result<(JsonValue, bool), &'static str>;

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
        Call::ComputeBudgetResetAt { settings, duration } => {
            computed_at(settings, duration, context)
        }
        Call::GetBudgetResetTime { settings, duration } => reset_time(settings, duration, context),
        Call::IsPersistable { settings, duration } => persistable(settings, duration, context),
        Call::BudgetDurationError { settings, duration } => {
            duration_error(settings, duration.as_deref(), context)
        }
    };
    match answer {
        Ok((value, warned)) => accepted(value, warned),
        Err(category) => rejected(category),
    }
}

fn parse_time(value: &JsonValue) -> Answer {
    parse_budget_reset_time(value)
        .map(|time| {
            (
                JsonValue::String(time.format("%H:%M:%S").to_string()),
                false,
            )
        })
        .map_err(|error| time_error_category(&error))
}

fn current_settings(settings: &RawSettings) -> Answer {
    get_budget_reset_settings(&settings.timezone, &settings.reset_time_of_day)
        .map(|resolved| (settings_json(&resolved), false))
        .map_err(|error| settings_error_category(&error))
}

fn computed_at(settings: &RawSettings, duration: &str, context: &RunContext<'_>) -> Answer {
    let current = get_budget_reset_settings(&settings.timezone, &settings.reset_time_of_day)
        .map_err(|error| settings_error_category(&error))?;
    compute_budget_reset_at(
        duration,
        &current,
        context.clocks.reset_now,
        context.database,
    )
    .map(|reset| reset_answer(&reset))
    .map_err(|error| duration_category(&error))
}

fn reset_time(settings: &RawSettings, duration: &str, context: &RunContext<'_>) -> Answer {
    let raw = settings.borrowed();
    get_budget_reset_time(duration, raw, context.clocks.reset_now, context.database)
        .map(|reset| reset_answer(&reset))
        .map_err(|error| reset_error_category(&error))
}

fn persistable(settings: &RawSettings, duration: &str, context: &RunContext<'_>) -> Answer {
    let raw = settings.borrowed();
    is_persistable_budget_duration(duration, raw, context.clocks, context.database)
        .map(|persistable| (JsonValue::Bool(persistable), false))
        .map_err(|error| reset_error_category(&error))
}

fn duration_error(
    settings: &RawSettings,
    duration: Option<&str>,
    context: &RunContext<'_>,
) -> Answer {
    match budget_duration_error(
        duration,
        settings.borrowed(),
        context.clocks,
        context.database,
    ) {
        Ok(None) => Ok((JsonValue::Null, false)),
        Ok(Some(_diagnostic)) => Ok((JsonValue::String("invalid".to_owned()), false)),
        Err(error) => Err(reset_error_category(&error)),
    }
}

fn reset_answer(reset: &Reset) -> (JsonValue, bool) {
    let micros = reset.instant.timestamp_micros();
    let warning_present = warned(&reset.warnings);
    (JsonValue::from(micros), warning_present)
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

const fn accepted(value: JsonValue, warned: bool) -> Outcome {
    Outcome {
        ok: true,
        value: Some(value),
        error: None,
        warned,
    }
}

const fn rejected(error: &'static str) -> Outcome {
    Outcome {
        ok: false,
        value: None,
        error: Some(error),
        warned: false,
    }
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

const fn reset_error_category(error: &BudgetResetError) -> &'static str {
    match error {
        BudgetResetError::Duration(duration) => duration_category(duration),
        BudgetResetError::Settings(settings) => settings_error_category(settings),
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
