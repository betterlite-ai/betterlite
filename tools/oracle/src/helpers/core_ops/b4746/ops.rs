use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_core::api::mcp_client_resources::list::{
    Prompts, Resources, Templates, list_from_session,
};
use betterlite_core::api::mcp_client_resources::session::ResourceSession as _;
use betterlite_core::api::mcp_client_resources::types::GetPromptRequestParams;
use betterlite_mcp::client::sessions::error::McpFailure;
use serde_json::json;
use url::Url;

use crate::helpers::{HelperError, HelperResult};

use super::input::{ErrorInput, Input, Operation};
use super::session::{FakeSession, Locked as _, Script, Step};

const SERVER: &str = "https://mcp.example.com/mcp";

pub(crate) fn run(input: &Input) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .map_err(failure)?;
    runtime.block_on(execute(input))
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

async fn execute(input: &Input) -> HelperResult {
    let script = Arc::new(Mutex::new(script_for(input)?));
    let mut session = FakeSession {
        script: Arc::clone(&script),
    };
    let deadline = Duration::from_millis(input.client_timeout_ms.max(input.listing_timeout_ms));
    macro_rules! listed {
        ($kind:ty) => {{
            let items =
                list_from_session::<_, $kind>(&mut session, deadline, input.raise_on_error, SERVER)
                    .await
                    .map_err(failure)?;
            let cursors = script.locked().cursors.clone();
            return Ok(json!({
                "items": serde_json::to_value(items).map_err(failure)?,
                "cursors": cursors
            }));
        }};
    }
    match input.operation {
        Operation::ListPrompts => listed!(Prompts),
        Operation::ListResources => listed!(Resources),
        Operation::ListResourceTemplates => listed!(Templates),
        Operation::GetPrompt => prompt(&mut session, input, &script).await,
        Operation::ReadResource => resource(&mut session, input, &script).await,
    }
}

fn script_for(input: &Input) -> Result<Script, Box<HelperError>> {
    let mut script = Script::new(input.capabilities.clone());
    script.pages = input.pages.iter().cloned().collect();
    match input.operation {
        Operation::GetPrompt => {
            let prompt = input
                .prompt
                .as_ref()
                .ok_or_else(|| failure("the case has no prompt input"))?;
            script.prompt_result = Some(step(prompt.error.as_ref(), prompt.result.as_ref())?);
        }
        Operation::ReadResource => {
            let resource = input
                .resource
                .as_ref()
                .ok_or_else(|| failure("the case has no resource input"))?;
            script.resource_result = Some(step(resource.error.as_ref(), resource.result.as_ref())?);
        }
        Operation::ListPrompts | Operation::ListResources | Operation::ListResourceTemplates => {}
    }
    Ok(script)
}

macro_rules! detail_operation {
    ($method:ident, $call:ident, $request:expr, $record:ident, $key:literal) => {
        async fn $method(
            session: &mut FakeSession,
            input: &Input,
            script: &Arc<Mutex<Script>>,
        ) -> HelperResult {
            let request = $request(input)?;
            let result = session.$call(request).await.map_err(failure)?;
            let recorded = script.locked().$record.clone();
            Ok(json!({"result": result, $key: recorded}))
        }
    };
}

detail_operation!(
    prompt,
    get_prompt,
    |input: &Input| -> Result<GetPromptRequestParams, Box<HelperError>> {
        let prompt = input
            .prompt
            .as_ref()
            .ok_or_else(|| failure("the case has no prompt input"))?;
        Ok(GetPromptRequestParams {
            name: prompt.name.clone(),
            arguments: prompt.arguments.clone(),
        })
    },
    prompt_params,
    "params"
);
detail_operation!(
    resource,
    read_resource,
    |input: &Input| -> Result<Url, Box<HelperError>> {
        let resource = input
            .resource
            .as_ref()
            .ok_or_else(|| failure("the case has no resource input"))?;
        Url::parse(&resource.uri).map_err(failure)
    },
    resource_uris,
    "uris"
);

fn step<Value>(
    scripted_error: Option<&ErrorInput>,
    result: Option<&Value>,
) -> Result<Step<Value>, Box<HelperError>>
where
    Value: Clone,
{
    if let Some(error) = scripted_error {
        return Ok(Step::Failure {
            failure: protocol_failure(error),
            delay: Duration::ZERO,
        });
    }
    let value = result
        .cloned()
        .ok_or_else(|| failure("the case has no result"))?;
    Ok(Step::Value {
        value,
        delay: Duration::ZERO,
    })
}

fn protocol_failure(error: &ErrorInput) -> McpFailure {
    McpFailure {
        code: error.code,
        message: error.message.clone(),
        context: None,
    }
}
