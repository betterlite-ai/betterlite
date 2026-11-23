use std::sync::Arc;

use betterlite_azure::azure_thread_messages::error::ThreadMessagesError;
use betterlite_azure::azure_thread_messages::operations::{ThreadMessagesApi, ThreadOperation};
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::records::ThreadCreate;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::azure_resource_dispatch::assistants::ThreadsDispatch;
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use serde_json::{Value, json};

use super::super::support::{
    AssistantsFake, OTHER, Reply as FakeReply, failure, runtime, with_status,
};
use super::{Answer, Fixture, Reply, Step};

const CREATE_TEXT: &str = r#"{"messages":[{"role":"user","content":"hello"}]}"#;

// B687's thread create and retrieve over the same fake transport
pub(super) fn answer(step: &Step) -> Answer {
    let (Step::ThreadCreate(reply) | Step::ThreadGet(reply)) = step else {
        return Err(failure(OTHER));
    };
    let runtime = runtime()?;
    let fake = Arc::new(AssistantsFake::new(vec![FakeReply::json(
        reply.status,
        &reply.body,
    )]));
    let fixture = Fixture::new(&reply.api_key, &reply.api_base, &reply.api_version);
    let request = fixture.request();
    let api = ThreadMessagesApi {
        transport: Arc::clone(&fake),
    };
    let create: ThreadCreate = serde_json::from_str(CREATE_TEXT).map_err(failure)?;
    let (kind, call) = call_of(step, reply, &create)?;
    let result = runtime.block_on(dispatch(AzureResourceOperation::AssistantThreads(
        Box::pin(
            ThreadsDispatch {
                api: &api,
                request: &request,
                supplied: None,
                flag: None,
                call,
            }
            .run(),
        ),
    )));
    Ok((kind, value_of(result)?))
}

fn call_of<'call>(
    step: &'call Step,
    reply: &'call Reply,
    create: &'call ThreadCreate,
) -> Result<(&'static str, ThreadOperation<'call>), Box<HelperError>> {
    match step {
        Step::ThreadCreate(_) => Ok(("thread_create", ThreadOperation::Create(create))),
        Step::ThreadGet(_) => Ok(("thread_get", ThreadOperation::Retrieve(&reply.target_id))),
        Step::DefinitionsCreate(_)
        | Step::DefinitionsList(_)
        | Step::DefinitionsDelete(_)
        | Step::MessageCreate(_)
        | Step::MessageList(_)
        | Step::RunPoll(_)
        | Step::RunStream(_) => Err(failure(OTHER)),
    }
}

fn value_of(
    result: Result<AzureResourceResult, AzureResourceError>,
) -> Result<Value, Box<HelperError>> {
    match result {
        Ok(AzureResourceResult::AssistantThreads(reply)) => Ok(json!({"id": reply.thread.id})),
        Ok(_) => Err(failure(OTHER)),
        Err(AzureResourceError::AssistantThreads(ThreadMessagesError::Threads(
            ThreadsError::Status { status, .. },
        ))) => Err(with_status(failure("the fixture failed"), status)),
        Err(error) => Err(failure(error)),
    }
}
