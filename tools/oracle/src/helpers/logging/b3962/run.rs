use std::collections::BTreeMap;

use betterlite_logging::braintrust_logging::event::{CallbackPath, ResponseObject, SuccessEvent};
use betterlite_logging::braintrust_logging::logger::BraintrustLogger;
use betterlite_logging::braintrust_logging::settings::Settings;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::helpers::{HelperError, HelperResult};

use super::capture::Capture;
use super::scenario::{InitScenario, LogScenario, Path, ProjectScenario, ResponseKind};

pub(super) fn init(scenarios: &[InitScenario]) -> HelperResult {
    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        let settings = settings(
            &scenario.environment,
            scenario.api_key.as_deref(),
            scenario.api_base.as_deref(),
            &scenario.default_tags,
        )?;
        results.push(json!({
            "api_base": settings.api_base,
            "api_key": settings.api_key.expose_secret(),
        }));
    }
    Ok(json!({"results": results}))
}

pub(super) fn projects(scenarios: &[ProjectScenario]) -> HelperResult {
    let runtime = runtime()?;
    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        results.push(runtime.block_on(project_scenario(scenario))?);
    }
    Ok(json!({"results": results}))
}

async fn project_scenario(scenario: &ProjectScenario) -> Result<Value, Box<HelperError>> {
    let settings = settings(
        &scenario.environment,
        scenario.api_key.as_deref(),
        scenario.api_base.as_deref(),
        &scenario.default_tags,
    )?;
    let capture = Capture::new(&scenario.answers);
    let logger = BraintrustLogger::new(settings, capture.clone());
    let mut ids = Vec::new();
    for _ in 0..scenario.calls {
        let id = logger
            .resolve_project(&scenario.project_name, callback_path(scenario.path))
            .await
            .map_err(|error| helper("BraintrustError", &error.to_string()))?;
        ids.push(Value::String(id));
    }
    Ok(json!({"requests": capture.requests(), "project_ids": ids}))
}

pub(super) fn logs(scenarios: &[LogScenario]) -> HelperResult {
    let runtime = runtime()?;
    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        results.push(runtime.block_on(log_scenario(scenario))?);
    }
    Ok(json!({"results": results}))
}

async fn log_scenario(scenario: &LogScenario) -> Result<Value, Box<HelperError>> {
    let settings = settings(
        &scenario.environment,
        scenario.api_key.as_deref(),
        scenario.api_base.as_deref(),
        &scenario.default_tags,
    )?;
    let capture = Capture::new(&scenario.answers);
    let logger = BraintrustLogger::new(settings, capture.clone());
    let event = event(scenario)?;
    for _ in 0..scenario.calls {
        logger
            .log_success_event(&event, callback_path(scenario.path))
            .await
            .map_err(|error| helper("BraintrustError", &error.to_string()))?;
    }
    Ok(json!({"requests": capture.requests()}))
}

fn event(scenario: &LogScenario) -> Result<SuccessEvent<'_>, Box<HelperError>> {
    let response = scenario.response.as_ref().map(|input| match input.kind {
        ResponseKind::ModelResponse => ResponseObject::Model(&input.body),
        ResponseKind::TextCompletion => ResponseObject::TextCompletion(&input.body),
        ResponseKind::Embedding => ResponseObject::Embedding(&input.body),
        ResponseKind::Image => ResponseObject::Image(&input.body),
    });
    let api_call_start = scenario
        .api_call_start_time
        .as_deref()
        .map(parse_time)
        .transpose()?;
    let completion_start = scenario
        .completion_start_time
        .as_deref()
        .map(parse_time)
        .transpose()?;
    Ok(SuccessEvent {
        call_id: scenario.call_id.as_ref(),
        messages: scenario.messages.as_ref(),
        standard_logging_object: scenario.standard_logging_object.as_ref(),
        metadata: scenario.metadata.as_ref(),
        response_cost: scenario.response_cost.as_ref(),
        call_type: scenario.call_type.as_deref(),
        response,
        api_call_start_time: api_call_start,
        completion_start_time: completion_start,
        start_time: parse_time(&scenario.start_time)?,
        end_time: parse_time(&scenario.end_time)?,
    })
}

fn settings(
    environment: &BTreeMap<String, String>,
    api_key: Option<&str>,
    api_base: Option<&str>,
    default_tags: &[String],
) -> Result<Settings, Box<HelperError>> {
    Settings::resolve(
        api_key.map(|key| SecretString::from(key.to_owned())),
        api_base.map(str::to_owned),
        default_tags.to_vec(),
        environment,
    )
    .map_err(|error| helper("SettingsError", &error.to_string()))
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper("RuntimeError", &error.to_string()))
}

fn parse_time(text: &str) -> Result<DateTime<Utc>, Box<HelperError>> {
    DateTime::parse_from_rfc3339(text)
        .map(|moment| moment.with_timezone(&Utc))
        .map_err(|error| helper("ValueError", &error.to_string()))
}

const fn callback_path(path: Path) -> CallbackPath {
    match path {
        Path::Sync => CallbackPath::Sync,
        Path::Async => CallbackPath::Async,
    }
}

fn helper(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
