use std::sync::Arc;

use betterlite_azure::assistants::error::AssistantsRunError;
use betterlite_azure::assistants::operations::{AssistantsRunsApi, RunCall, RunDispatch};
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::records::RunRequest;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::azure_resource_dispatch::assistants::{
    self, RunEvents, RunOutcome, RunsDispatch,
};
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use futures_util::StreamExt as _;
use serde_json::json;
use tokio::runtime::Runtime;

use super::super::support::{AssistantsFake, OTHER, failure, runtime, with_status};
use super::{Answer, Fixture, Planned, Step};

const RUN_TEXT: &str = r#"{"thread_id":"thread_1","assistant_id":"asst_1","model":"gpt-4o-mini","instructions":"base"}"#;

// B543's buffered and streaming runs; the streaming run keeps the SDK stream's event names
pub(super) fn answer(step: &Step) -> Answer {
    let fixture = match step {
        Step::RunPoll(value) | Step::RunStream(value) => value,
        _ => return Err(failure(OTHER)),
    };
    let runtime = runtime()?;
    let fake = Arc::new(AssistantsFake::new(
        fixture.replies.iter().map(Planned::planned).collect(),
    ));
    let request_fixture = Fixture::new(&fixture.api_key, &fixture.api_base, &fixture.api_version);
    let request = request_fixture.request();
    let api = AssistantsRunsApi {
        transport: Arc::clone(&fake),
    };
    let run: RunRequest = serde_json::from_str(RUN_TEXT).map_err(failure)?;
    let streaming = matches!(step, Step::RunStream(_));
    let result = runtime.block_on(dispatch(AzureResourceOperation::AssistantRuns(Box::pin(
        RunsDispatch {
            api: &api,
            request: &request,
            supplied: None,
            call: RunCall {
                dispatch: RunDispatch {
                    arun_thread: None,
                    stream: streaming.then_some(true),
                },
                run: &run,
                handler: None,
            },
        }
        .run(),
    ))));
    match result {
        Ok(AzureResourceResult::AssistantRuns(RunOutcome::Polled(_kind, reply))) => {
            Ok(("run_poll", json!({"status": reply.status})))
        }
        Ok(AzureResourceResult::AssistantRuns(RunOutcome::Streaming(_kind, events))) => Ok((
            "run_stream",
            json!({"events": event_names(&runtime, events)?}),
        )),
        Ok(_) => Err(failure(OTHER)),
        Err(AzureResourceError::AssistantRuns(AssistantsRunError::Runs(
            ThreadRunsError::Status(status, _body),
        ))) => Err(with_status(failure("the fixture failed"), status)),
        Err(error) => Err(failure(error)),
    }
}

fn event_names(runtime: &Runtime, events: RunEvents) -> Result<Vec<String>, Box<HelperError>> {
    let mut stream = runtime.block_on(events).map_err(failure)?;
    let mut names: Vec<String> = Vec::new();
    runtime.block_on(async {
        while let Some(event) = stream.next().await {
            names.push(event.map_err(failure)?.name().to_owned());
        }
        Ok::<(), Box<HelperError>>(())
    })?;
    Ok(names)
}
