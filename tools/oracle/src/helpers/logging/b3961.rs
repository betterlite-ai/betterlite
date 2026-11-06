use std::collections::BTreeMap;

use betterlite_logging::http_analytics_loggers::athina::{AthinaLogger, AthinaSettings};
use betterlite_logging::http_analytics_loggers::event::Event;
use betterlite_logging::http_analytics_loggers::greenscale::{
    GreenscaleLogger, GreenscaleSettings,
};
use betterlite_logging::http_analytics_loggers::promptlayer::{
    PromptLayerLogger, PromptLayerSettings,
};
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

use self::capture::{Answer, Capture, FixedClock, Recorded};
use self::init::{athina_init, greenscale_init, promptlayer_init};

mod capture;
mod init;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    AthinaInit { scenarios: Vec<InitScenario> },
    AthinaLog { scenarios: Vec<LogScenario> },
    GreenscaleInit { scenarios: Vec<InitScenario> },
    GreenscaleLog { scenarios: Vec<LogScenario> },
    PromptlayerInit { scenarios: Vec<InitScenario> },
    PromptlayerLog { scenarios: Vec<LogScenario> },
    Log { scenarios: Vec<LogScenario> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InitScenario {
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LoggerName {
    Athina,
    Greenscale,
    Promptlayer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogScenario {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    logger: Option<LoggerName>,
    #[serde(default)]
    kwargs: Option<Json>,
    #[serde(default)]
    response: Option<Json>,
    #[serde(default)]
    complete_streaming_response: Option<Json>,
    #[serde(default)]
    now: Option<String>,
    start_time: String,
    end_time: String,
    #[serde(default)]
    answers: Vec<Answer>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::AthinaInit { scenarios } => Ok(json!({
                "results": scenarios.iter().map(athina_init).collect::<Vec<_>>(),
            })),
            Input::AthinaLog { scenarios } => run_logs(scenarios, Some(LoggerName::Athina)),
            Input::GreenscaleInit { scenarios } => Ok(json!({
                "results": scenarios.iter().map(greenscale_init).collect::<Vec<_>>(),
            })),
            Input::GreenscaleLog { scenarios } => run_logs(scenarios, Some(LoggerName::Greenscale)),
            Input::PromptlayerInit { scenarios } => Ok(json!({
                "results": scenarios.iter().map(promptlayer_init).collect::<Vec<_>>(),
            })),
            Input::PromptlayerLog { scenarios } => {
                run_logs(scenarios, Some(LoggerName::Promptlayer))
            }
            Input::Log { scenarios } => run_logs(scenarios, None),
        }
    }
}

fn run_logs(scenarios: &[LogScenario], forced: Option<LoggerName>) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        let logger = forced.or(scenario.logger).unwrap_or(LoggerName::Athina);
        results.push(runtime.block_on(log_scenario(scenario, logger))?);
    }
    Ok(json!({"results": results}))
}

async fn log_scenario(
    scenario: &LogScenario,
    logger: LoggerName,
) -> Result<serde_json::Value, Box<HelperError>> {
    let kwargs = buffered_kwargs(scenario)?;
    let start = parse_time(&scenario.start_time)?;
    let end = parse_time(&scenario.end_time)?;
    let event = Event {
        kwargs: &kwargs,
        response: scenario.response.as_ref(),
        start_time: start,
        end_time: end,
    };
    let capture = Capture::with_answers(scenario.answers.clone());
    dispatch(scenario, logger, &event, &capture).await?;
    let recorded = capture
        .requests()
        .map_err(|error| helper_error("CaptureError", &error.to_string()))?;
    Ok(json!({
        "requests": recorded.iter().map(record_json).collect::<Vec<_>>(),
        "raised": false,
    }))
}

fn buffered_kwargs(scenario: &LogScenario) -> Result<Json, Box<HelperError>> {
    let mut kwargs = scenario
        .kwargs
        .clone()
        .ok_or_else(|| helper_error("InvalidInput", "every logged scenario carries kwargs"))?;
    if let Some(complete) = &scenario.complete_streaming_response {
        let Json::Object(fields) = &mut kwargs else {
            return Err(helper_error("InvalidInput", "kwargs must be an object"));
        };
        fields.insert("complete_streaming_response".to_owned(), complete.clone());
    }
    Ok(kwargs)
}

async fn dispatch(
    scenario: &LogScenario,
    logger: LoggerName,
    event: &Event<'_>,
    capture: &Capture,
) -> Result<(), Box<HelperError>> {
    match logger {
        LoggerName::Athina => {
            AthinaLogger {
                delivery: capture.clone(),
                settings: AthinaSettings::from_environment(&scenario.environment),
            }
            .log_event(event)
            .await;
        }
        LoggerName::Greenscale => {
            let now = match &scenario.now {
                Some(text) => parse_time(text)?,
                None => event.start_time,
            };
            GreenscaleLogger {
                delivery: capture.clone(),
                clock: FixedClock { now },
                settings: GreenscaleSettings::from_environment(&scenario.environment),
            }
            .log_event(event)
            .await;
        }
        LoggerName::Promptlayer => {
            PromptLayerLogger {
                delivery: capture.clone(),
                settings: PromptLayerSettings::from_environment(&scenario.environment),
            }
            .log_event(event)
            .await;
        }
    }
    Ok(())
}

fn record_json(recorded: &Recorded) -> serde_json::Value {
    let headers: IndexMap<String, Json> = recorded
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    json!({"url": recorded.url, "headers": headers, "body": recorded.body})
}

fn parse_time(text: &str) -> Result<DateTime<Utc>, Box<HelperError>> {
    DateTime::parse_from_rfc3339(text)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            helper_error(
                "InvalidInput",
                &format!("{text} is not an RFC 3339 time: {error}"),
            )
        })
}

fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    let error = HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    };
    Box::new(error)
}
