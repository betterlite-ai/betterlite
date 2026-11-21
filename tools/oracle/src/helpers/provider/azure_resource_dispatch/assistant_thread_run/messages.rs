use std::sync::Arc;

use betterlite_azure::azure_assistant_definitions::operations::AssistantsApi;
use betterlite_azure::azure_message_attachments::error::MessagesError;
use betterlite_azure::azure_message_attachments::operations::{
    MessageCall, MessageReply, MessagesApi,
};
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::records::ThreadMessageData;
use betterlite_provider::azure_resource_dispatch::assistants::{self, MessagesDispatch};
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use serde_json::json;

use super::super::support::{
    AssistantsFake, OTHER, Reply as FakeReply, failure, runtime, with_status,
};
use super::{Answer, Fixture, Step};

const MESSAGE_TEXT: &str = r#"{"role":"user","content":"hello"}"#;

// B697's message append and list, the append path carrying the attachment policy
pub(super) fn answer(step: &Step) -> Answer {
    let reply = match step {
        Step::MessageCreate(value) | Step::MessageList(value) => value,
        _ => return Err(failure(OTHER)),
    };
    let runtime = runtime()?;
    let fake = Arc::new(AssistantsFake::new(vec![FakeReply::json(
        reply.status,
        &reply.body,
    )]));
    let fixture = Fixture::new(&reply.api_key, &reply.api_base, &reply.api_version);
    let request = fixture.request();
    let api = MessagesApi {
        assistants: AssistantsApi {
            transport: Arc::clone(&fake),
        },
    };
    let data: ThreadMessageData = serde_json::from_str(MESSAGE_TEXT).map_err(failure)?;
    let (kind, call) = match step {
        Step::MessageCreate(_) => (
            "message_create",
            MessageCall::Append {
                thread_id: &reply.target_id,
                data: &data,
            },
        ),
        Step::MessageList(_) => ("message_list", MessageCall::List(&reply.target_id)),
        _ => return Err(failure(OTHER)),
    };
    let result = runtime.block_on(dispatch(AzureResourceOperation::AssistantMessages(
        Box::pin(
            MessagesDispatch {
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
        Ok(AzureResourceResult::AssistantMessages(reply)) => match reply.reply {
            MessageReply::Message(message) => json!({"id": message.id}),
            MessageReply::Page(page) => json!({"count": page.data.len()}),
        },
        Ok(_) => return Err(failure(OTHER)),
        Err(AzureResourceError::AssistantMessages(MessagesError::Threads(
            ThreadsError::Status { status, .. },
        ))) => return Err(with_status(failure("the fixture failed"), status)),
        Err(error) => return Err(failure(error)),
    };
    Ok((kind, value))
}
