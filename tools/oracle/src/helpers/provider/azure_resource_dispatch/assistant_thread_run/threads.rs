use std::sync::Arc;

use betterlite_azure::azure_thread_messages::error::ThreadMessagesError;
use betterlite_azure::azure_thread_messages::operations::{ThreadMessagesApi, ThreadOperation};
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::records::ThreadCreate;
use betterlite_provider::azure_resource_dispatch::assistants::ThreadsDispatch;
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use serde_json::json;

use super::super::support::{
    AssistantsFake, OTHER, Reply as FakeReply, failure, runtime, with_status,
};
use super::{Answer, Fixture, Step};

const CREATE_TEXT: &str = r#"{"messages":[{"role":"user","content":"hello"}]}"#;

// B687's thread create and retrieve over the same fake transport
pub(super) fn answer(step: &Step) -> Answer {
    let reply = match step {
        Step::ThreadCreate(value) | Step::ThreadGet(value) => value,
        _ => return Err(failure(OTHER)),
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
    let (kind, call) = match step {
        Step::ThreadCreate(_) => ("thread_create", ThreadOperation::Create(&create)),
        Step::ThreadGet(_) => ("thread_get", ThreadOperation::Retrieve(&reply.target_id)),
        _ => return Err(failure(OTHER)),
    };
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
    let value = match result {
        Ok(AzureResourceResult::AssistantThreads(reply)) => json!({"id": reply.thread.id}),
        Ok(_) => return Err(failure(OTHER)),
        Err(AzureResourceError::AssistantThreads(ThreadMessagesError::Threads(
            ThreadsError::Status { status, .. },
        ))) => return Err(with_status(failure("the fixture failed"), status)),
        Err(error) => return Err(failure(error)),
    };
    Ok((kind, value))
}
