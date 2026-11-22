use std::sync::Arc;

use betterlite_azure::azure_assistant_definitions::operations::AssistantsApi;
use betterlite_azure::azure_message_attachments::error::MessagesError;
use betterlite_azure::azure_message_attachments::operations::{
    MessageCall, MessageReply, MessagesApi,
};
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::records::ThreadMessageData;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::azure_resource_dispatch::assistants::MessagesDispatch;
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use serde_json::{Value, json};

use super::super::support::{
    AssistantsFake, OTHER, Reply as FakeReply, failure, runtime, with_status,
};
use super::{Answer, Fixture, Reply, Step};

const MESSAGE_TEXT: &str = r#"{"role":"user","content":"hello"}"#;

// B697's message append and list, the append path carrying the attachment policy
pub(super) fn answer(step: &Step) -> Answer {
    let (Step::MessageCreate(reply) | Step::MessageList(reply)) = step else {
        return Err(failure(OTHER));
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
    let (kind, call) = call_of(step, reply, &data)?;
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
    Ok((kind, value_of(result)?))
}

fn call_of<'call>(
    step: &'call Step,
    reply: &'call Reply,
    data: &'call ThreadMessageData,
) -> Result<(&'static str, MessageCall<'call>), Box<HelperError>> {
    match step {
        Step::MessageCreate(_) => Ok((
            "message_create",
            MessageCall::Append {
                thread_id: &reply.target_id,
                data,
            },
        )),
        Step::MessageList(_) => Ok(("message_list", MessageCall::List(&reply.target_id))),
        Step::DefinitionsCreate(_)
        | Step::DefinitionsList(_)
        | Step::DefinitionsDelete(_)
        | Step::ThreadCreate(_)
        | Step::ThreadGet(_)
        | Step::RunPoll(_)
        | Step::RunStream(_) => Err(failure(OTHER)),
    }
}

fn value_of(
    result: Result<AzureResourceResult, AzureResourceError>,
) -> Result<Value, Box<HelperError>> {
    match result {
        Ok(AzureResourceResult::AssistantMessages(reply)) => match reply.reply {
            MessageReply::Message(message) => Ok(json!({"id": message.id})),
            MessageReply::Page(page) => Ok(json!({"count": page.data.len()})),
        },
        Ok(_) => Err(failure(OTHER)),
        Err(AzureResourceError::AssistantMessages(MessagesError::Threads(
            ThreadsError::Status { status, .. },
        ))) => Err(with_status(failure("the fixture failed"), status)),
        Err(error) => Err(failure(error)),
    }
}
