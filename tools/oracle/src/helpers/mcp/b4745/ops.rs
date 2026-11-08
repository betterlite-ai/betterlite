use std::error::Error;
use std::fmt::Display;
use std::time::Duration;

use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::tools::calling::call_tool;
use betterlite_mcp::client::tools::listing::list_tools;
use betterlite_mcp::client::tools::result::{ContentBlock, ToolCallResult};
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::fixtures::{
    BrokenResourceError, CallMode, RecordingHost, Script, Simulated, SimulatedError, build_client,
    host_for,
};
use super::input::{CallScenario, ErrorKind, Input, ListScenario, Settings};
use super::logs::Logs;

pub(crate) fn failure<Source>(error: Source) -> Box<HelperError>
where
    Source: Display,
{
    let message = error.to_string();
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        message,
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}

pub(crate) async fn run(input: &Input, logs: &Logs) -> HelperResult {
    let results = match input {
        Input::ListTools {
            settings,
            scenarios,
        } => {
            let mut results = Vec::with_capacity(scenarios.len());
            for scenario in scenarios {
                results.push(value(settings, ScenarioInput::List(scenario), logs).await);
            }
            results
        }
        Input::CallTool {
            settings,
            scenarios,
        } => {
            let mut results = Vec::with_capacity(scenarios.len());
            for scenario in scenarios {
                results.push(value(settings, ScenarioInput::Call(scenario), logs).await);
            }
            results
        }
    };
    Ok(json!({ "results": results }))
}

#[derive(Clone, Copy)]
enum ScenarioInput<'scenario> {
    List(&'scenario ListScenario),
    Call(&'scenario CallScenario),
}

async fn value(settings: &Settings, input: ScenarioInput<'_>, logs: &Logs) -> Value {
    let error = match input {
        ScenarioInput::List(scenario) => scenario.error,
        ScenarioInput::Call(scenario) => scenario.error,
    };
    match error {
        Some(ErrorKind::Broken) => {
            scenario_value::<BrokenResourceError>(settings, input, logs).await
        }
        None | Some(ErrorKind::Plain) => {
            scenario_value::<SimulatedError>(settings, input, logs).await
        }
    }
}

async fn scenario_value<Source>(settings: &Settings, input: ScenarioInput<'_>, logs: &Logs) -> Value
where
    Source: Simulated,
{
    logs.reset();
    match input {
        ScenarioInput::List(scenario) => {
            let mut client =
                build_client(settings, Script::<Source>::new(CallMode::Listing(scenario)));
            let listing = Duration::from_millis(settings.listing_ms);
            let outcome = list_tools(&mut client, listing, scenario.raise_on_error).await;
            outcome.map_or_else(
                |_| {
                    let error = if scenario.cancel {
                        "cancelled"
                    } else {
                        "error"
                    };
                    json!({ "outcome": "raised", "error": error, "tools": Value::Null, "logs": logs.entries() })
                },
                |tools| {
                    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
                    json!({ "outcome": "ok", "tools": names, "logs": logs.entries() })
                },
            )
        }
        ScenarioInput::Call(scenario) => {
            let host = host_for(scenario.host);
            let mut client =
                build_client(settings, Script::<Source>::new(CallMode::Calling(scenario)));
            let outcome = call_tool(
                &mut client,
                &scenario.request(),
                host.as_ref(),
                scenario.options(),
            )
            .await;
            call_value(outcome, host.as_ref(), &logs.entries())
        }
    }
}

fn call_value<Source>(
    outcome: Result<ToolCallResult, SessionError<Source>>,
    host: Option<&RecordingHost>,
    logs: &[(String, String)],
) -> Value
where
    Source: Error + Send + Sync + 'static,
{
    let progress = host.map(RecordingHost::seen).unwrap_or_default();
    match outcome {
        Ok(result) => json!({
            "outcome": "ok",
            "result": result_value(&result),
            "progress": progress,
            "logs": logs,
        }),
        Err(error) => json!({
            "outcome": "raised",
            "error": error_kind(&error),
            "result": Value::Null,
            "progress": progress,
            "logs": logs,
        }),
    }
}

fn result_value(result: &ToolCallResult) -> Value {
    match result {
        ToolCallResult::Complete(complete) => json!({
            "kind": "complete",
            "is_error": complete.is_error,
            "content": complete
                .content
                .iter()
                .map(block_value)
                .collect::<Vec<_>>(),
        }),
        ToolCallResult::InputRequired(input) => json!({
            "kind": "input_required",
            "result_type": input.result_type,
            "request_state": input.request_state,
        }),
    }
}

fn block_value(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text(text) => json!({
            "type": "text",
            "text": text.text.clone(),
        }),
        ContentBlock::Image(_) => json!({ "type": "image" }),
        ContentBlock::Audio(_) => json!({ "type": "audio" }),
        ContentBlock::ResourceLink(_) => json!({ "type": "resource_link" }),
        ContentBlock::Resource(_) => json!({ "type": "resource" }),
    }
}

const fn error_kind<Source>(error: &SessionError<Source>) -> &'static str
where
    Source: Error + Send + Sync + 'static,
{
    match error {
        SessionError::Cancelled | SessionError::CleanupTimedOut => "cancelled",
        SessionError::Protocol(_)
        | SessionError::Runtime(_)
        | SessionError::Stream(_)
        | SessionError::CleanupTask(_)
        | SessionError::ReadTimeout(_)
        | SessionError::Configuration(_) => "error",
    }
}
