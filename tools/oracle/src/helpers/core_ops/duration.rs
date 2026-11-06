use betterlite_core::duration::reset::next_standardized_reset_time;
use betterlite_core::duration::seconds::duration_in_seconds;
use chrono::{DateTime, NaiveTime, Utc};
use jiff::tz::TimeZoneDatabase;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    DurationInSeconds,
    Reset,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    duration: String,
    now: String,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default = "default_reset_time_of_day")]
    reset_time_of_day: String,
}

#[derive(Serialize)]
struct Outcome {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<i64>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let database = TimeZoneDatabase::from_env();
        let outcomes = input
            .calls
            .iter()
            .map(|call| run_call(call, &database))
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_value(outcomes).map_err(failure)
    }
}

fn run_call(
    call: &Call,
    database: &TimeZoneDatabase,
) -> Result<Outcome, Box<crate::helpers::HelperError>> {
    let parsed_now = DateTime::parse_from_rfc3339(&call.now).map_err(failure)?;
    let now = parsed_now.with_timezone(&Utc);
    let time_of_day =
        NaiveTime::parse_from_str(&call.reset_time_of_day, "%H:%M:%S%.f").map_err(failure)?;
    let outcome = match call.function {
        Function::DurationInSeconds => {
            duration_in_seconds(&call.duration, now, call.timezone.as_deref(), database)
                .map(|result| result.seconds)
        }
        Function::Reset => next_standardized_reset_time(
            &call.duration,
            now,
            call.timezone.as_deref(),
            time_of_day,
            database,
        )
        .map(|result| result.instant.timestamp_micros()),
    };
    Ok(outcome.map_or(
        Outcome {
            ok: false,
            value: None,
        },
        |value| Outcome {
            ok: true,
            value: Some(value),
        },
    ))
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "DurationInput".to_owned(),
        message: error.to_string(),
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}

fn default_reset_time_of_day() -> String {
    "00:00:00".to_owned()
}
