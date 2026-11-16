use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_openai::realtime::logging::{RealtimeLogRecord, RealtimeLogger};
use betterlite_openai::thread_runs::operations::RunDispatch;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_assistant_dispatch::OpenAiAssistantRequest;
use betterlite_provider::openai_assistant_dispatch::call::{
    OpenAiAssistantOperation, RealtimeCall, RunsCall,
};
use betterlite_provider::openai_assistant_dispatch::execute::dispatch as compose;
use betterlite_provider::operation::{OperationRequest, OperationResult};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::assistant::assistants;
use super::decision::decisions;
use super::input::{Input, Operation, RealtimeOp, RunsOp};
use super::invalid_input;
use super::project::{failure, outcome};
use super::realtime::{connect, probe_failure};
use super::thread::threads;
use super::wire::Wire;

const ORG: &str = "org-fixture";

// The composition logs nothing itself, so the oracle's logger only has to accept the pre-call record.
struct Quiet;

impl RealtimeLogger for Quiet {
    fn pre_call(&mut self, _record: RealtimeLogRecord<'_>) -> Result<(), PrePostError> {
        Ok(())
    }
}

pub(crate) fn answer(input: &Input) -> HelperResult {
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let wire = Wire::replying(&input.replies);
    let first = runtime.block_on(exchange(operation(input)?, &wire))?;
    let result = match input.operation {
        // LiteLLM's probe connection fails once it is opened, so a realtime row also reports how the client socket closes
        Operation::Realtime(RealtimeOp::Connect) => {
            let probe =
                OpenAiAssistantOperation::Realtime(RealtimeCall::Failure(probe_failure(input)));
            let closed = runtime.block_on(exchange(probe, &wire))?;
            json!({"session": first, "closed": closed})
        }
        Operation::Assistants(_)
        | Operation::Threads(_)
        | Operation::Runs(_)
        | Operation::Decisions(_) => first,
    };
    Ok(json!({"requests": wire.seen(), "result": result}))
}

async fn exchange(
    operation: OpenAiAssistantOperation,
    wire: &Arc<Wire>,
) -> Result<Value, Box<HelperError>> {
    let reply = compose(
        OperationRequest::OpenAiAssistant(OpenAiAssistantRequest),
        operation,
        Arc::clone(wire),
        &mut Quiet,
    )
    .await;
    let sent_at_dispatch = wire.seen().len();
    match reply {
        Ok(OperationResult::OpenAiAssistant(result)) => outcome(result, sent_at_dispatch).await,
        Ok(_other) => Err(invalid_input(String::from(
            "the composition returned another operation",
        ))),
        Err(error) => Err(failure(&error)),
    }
}

fn operation(input: &Input) -> Result<OpenAiAssistantOperation, Box<HelperError>> {
    match input.operation {
        Operation::Assistants(operation) => assistants(input, operation),
        Operation::Threads(operation) => threads(input, operation),
        Operation::Runs(operation) => {
            let request = input
                .request
                .clone()
                .ok_or_else(|| invalid_input(String::from("request is required for a run")))?;
            let dispatch = RunDispatch {
                arun_thread: input.arun_thread,
                stream: matches!(operation, RunsOp::Stream),
            };
            Ok(OpenAiAssistantOperation::Runs(RunsCall {
                options: options(input),
                dispatch,
                request,
            }))
        }
        Operation::Decisions(operation) => decisions(input, operation),
        Operation::Realtime(RealtimeOp::Connect) => Ok(connect(input)),
    }
}

pub(super) fn options(input: &Input) -> ClientOptions {
    ClientOptions {
        api_key: input.api_key.clone(),
        api_base: input.api_base.clone(),
        timeout: Some(Duration::from_secs(30)),
        max_retries: Some(0),
        organization: Some(String::from(ORG)),
    }
}
