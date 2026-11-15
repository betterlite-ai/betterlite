use std::collections::BTreeMap;

use betterlite_db::database_diagnostics::json_enabled::{GlobalJsonLogs, is_json_enabled};
use betterlite_db::database_diagnostics::json_formatter::JsonFormatter;
use betterlite_db::database_diagnostics::legacy::TextFormatter;
use betterlite_db::database_diagnostics::message::{LogArguments, LogValue};
use betterlite_db::database_diagnostics::record::{ExceptionRecord, LogRecord};
use betterlite_db::database_diagnostics::timestamp::FixedLocalTime;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::FixedOffset;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    format_scenarios: Vec<FormatScenario>,
    #[serde(default)]
    enabled_scenarios: Vec<EnabledScenario>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Format,
    JsonEnabled,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FormatScenario {
    format: ScenarioFormat,
    message: String,
    level: String,
    created: f64,
    offset_seconds: i32,
    #[serde(default)]
    arguments: Option<RawArguments>,
    #[serde(default)]
    exception: Option<RawException>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioFormat {
    Json,
    Text,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum RawArguments {
    Positional(Vec<LogValue>),
    Named(BTreeMap<String, LogValue>),
}

impl RawArguments {
    fn arguments(&self) -> LogArguments {
        match self {
            Self::Positional(values) => LogArguments::Positional(values.clone()),
            Self::Named(values) => LogArguments::Named(values.clone()),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawException {
    kind: String,
    message: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EnabledScenario {
    mode: EnabledMode,
    #[serde(default)]
    json_logs: Option<String>,
    #[serde(default)]
    global_enabled: bool,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum EnabledMode {
    Attribute,
    MissingAttribute,
    ImportError,
    AttributeError,
    OtherError,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FormattedOutput {
    message: String,
    level: String,
    timestamp: String,
    #[serde(default)]
    stacktrace: Option<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::Format => format_scenarios(&input.format_scenarios),
            Function::JsonEnabled => enabled_scenarios(&input.enabled_scenarios),
        }
    }
}

fn format_scenarios(scenarios: &[FormatScenario]) -> HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| helper_error("ValueError", &error.to_string()))?;
    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        let offset = FixedOffset::east_opt(scenario.offset_seconds)
            .ok_or_else(|| helper_error("ValueError", "the timezone offset is out of range"))?;
        let local = FixedLocalTime { offset };
        let exception = scenario
            .exception
            .as_ref()
            .map(|exception| ExceptionRecord {
                kind: &exception.kind,
                message: &exception.message,
            });
        let arguments = scenario
            .arguments
            .as_ref()
            .map_or(LogArguments::None, RawArguments::arguments);
        let record = LogRecord {
            created: scenario.created,
            level: &scenario.level,
            message: &scenario.message,
            arguments,
            exception,
        };
        results.push(render(scenario.format, &local, &redactor, &record)?);
    }
    Ok(serde_json::Value::Array(results))
}

fn render(
    format: ScenarioFormat,
    local: &FixedLocalTime,
    redactor: &Redactor,
    record: &LogRecord<'_>,
) -> Result<serde_json::Value, Box<HelperError>> {
    match format {
        ScenarioFormat::Json => {
            let rendered = JsonFormatter {
                local_time: local,
                redactor,
            }
            .format(record)
            .map_err(|error| helper_error("ValueError", &error.to_string()))?;
            let parsed: FormattedOutput = serde_json::from_str(&rendered)?;
            Ok(serde_json::json!({
                "message": parsed.message,
                "level": parsed.level,
                "timestamp": parsed.timestamp,
                "stacktrace": parsed.stacktrace,
            }))
        }
        ScenarioFormat::Text => TextFormatter {
            local_time: local,
            redactor,
        }
        .format(record)
        .map(serde_json::Value::String)
        .map_err(|error| helper_error("ValueError", &error.to_string())),
    }
}

fn enabled_scenarios(scenarios: &[EnabledScenario]) -> HelperResult {
    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        let global = match scenario.mode {
            EnabledMode::Attribute => {
                GlobalJsonLogs::Supplied(LogValue::Bool(scenario.global_enabled))
            }
            EnabledMode::MissingAttribute => GlobalJsonLogs::Missing,
            EnabledMode::ImportError => GlobalJsonLogs::ImportError,
            EnabledMode::AttributeError => GlobalJsonLogs::AttributeError,
            EnabledMode::OtherError => GlobalJsonLogs::Failed,
        };
        let enabled = is_json_enabled(&global, scenario.json_logs.as_deref())
            .map_err(|error| helper_error("JsonLogsError", &error.to_string()))?;
        results.push(serde_json::Value::Bool(enabled));
    }
    Ok(serde_json::Value::Array(results))
}

fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
