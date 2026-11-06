use std::collections::BTreeMap;

use betterlite_logging::braintrust_logging::event::{CallbackPath, ResponseObject, SuccessEvent};
use betterlite_logging::braintrust_logging::logger::BraintrustLogger;
use betterlite_logging::braintrust_logging::settings::Settings;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

use self::capture::{Answer, Capture};

mod capture;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ResolveProject { scenarios: Vec<Scenario> },
    Log { scenarios: Vec<Scenario> },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Path {
    Sync,
    Async,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseKind {
    ModelResponse,
    TextCompletion,
    Embedding,
    Image,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseInput {
    kind: ResponseKind,
    body: Json,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    #[serde(default)]
    path: Option<Path>,
    #[serde(default)]
    project_name: Option<String>,
    #[serde(default)]
    start_time: Option<String>,
    #[serde(default)]
    end_time: Option<String>,
    #[serde(default = "default_calls")]
    calls: u32,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    default_tags: Vec<String>,
    #[serde(default)]
    call_id: Option<Json>,
    #[serde(default)]
    messages: Option<Json>,
    #[serde(default)]
    standard_logging_object: Option<Json>,
    #[serde(default)]
    metadata: Option<Json>,
    #[serde(default)]
    response_cost: Option<Json>,
    #[serde(default)]
    call_type: Option<String>,
    #[serde(default)]
    response: Option<ResponseInput>,
    #[serde(default)]
    api_call_start_time: Option<String>,
    #[serde(default)]
    completion_start_time: Option<String>,
    #[serde(default)]
    answers: Vec<Answer>,
}

const fn default_calls() -> u32 {
    1
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ResolveProject { scenarios } => collect(scenarios, Action::Project),
            Input::Log { scenarios } => collect(scenarios, Action::Log),
        }
    }
}

#[derive(Clone, Copy)]
enum Action {
    Project,
    Log,
}

fn collect(scenarios: &[Scenario], action: Action) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper("RuntimeError", &error.to_string()))?;
    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        results.push(runtime.block_on(run_scenario(scenario, action))?);
    }
    Ok(json!({"results": results}))
}

async fn run_scenario(scenario: &Scenario, action: Action) -> Result<Value, Box<HelperError>> {
    let capture = Capture::new(&scenario.answers);
    let logger = BraintrustLogger::new(settings(scenario)?, capture.clone());
    let path = callback_path(scenario.path)?;
    match action {
        Action::Project => {
            let ids = resolve_project(scenario, &logger, path).await?;
            Ok(json!({"requests": capture.requests(), "project_ids": ids}))
        }
        Action::Log => {
            log_events(scenario, &logger, path).await?;
            Ok(json!({"requests": capture.requests()}))
        }
    }
}

async fn resolve_project(
    scenario: &Scenario,
    logger: &BraintrustLogger<Capture>,
    path: CallbackPath,
) -> Result<Vec<Value>, Box<HelperError>> {
    let name = scenario
        .project_name
        .as_deref()
        .ok_or_else(|| helper("ValueError", "project_name must be a string"))?;
    let mut ids = Vec::new();
    for _ in 0..scenario.calls {
        let id = logger
            .resolve_project(name, path)
            .await
            .map_err(|error| helper("BraintrustError", &error.to_string()))?;
        ids.push(Value::String(id));
    }
    Ok(ids)
}

async fn log_events(
    scenario: &Scenario,
    logger: &BraintrustLogger<Capture>,
    path: CallbackPath,
) -> Result<(), Box<HelperError>> {
    let event = event(scenario)?;
    for _ in 0..scenario.calls {
        logger
            .log_success_event(&event, path)
            .await
            .map_err(|error| helper("BraintrustError", &error.to_string()))?;
    }
    Ok(())
}

fn event(scenario: &Scenario) -> Result<SuccessEvent<'_>, Box<HelperError>> {
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
    let start = scenario
        .start_time
        .as_deref()
        .ok_or_else(|| helper("ValueError", "start_time must be a string"))?;
    let end = scenario
        .end_time
        .as_deref()
        .ok_or_else(|| helper("ValueError", "end_time must be a string"))?;
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
        start_time: parse_time(start)?,
        end_time: parse_time(end)?,
    })
}

fn settings(scenario: &Scenario) -> Result<Settings, Box<HelperError>> {
    Settings::resolve(
        scenario
            .api_key
            .as_ref()
            .map(|key| SecretString::from(key.clone())),
        scenario.api_base.clone(),
        scenario.default_tags.clone(),
        &scenario.environment,
    )
    .map_err(|error| helper("SettingsError", &error.to_string()))
}

fn callback_path(path: Option<Path>) -> Result<CallbackPath, Box<HelperError>> {
    match path {
        Some(Path::Sync) => Ok(CallbackPath::Sync),
        Some(Path::Async) => Ok(CallbackPath::Async),
        None => Err(helper("ValueError", "path must be sync or async")),
    }
}

fn parse_time(text: &str) -> Result<DateTime<Utc>, Box<HelperError>> {
    DateTime::parse_from_rfc3339(text)
        .map(|moment| moment.with_timezone(&Utc))
        .map_err(|error| helper("ValueError", &error.to_string()))
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
