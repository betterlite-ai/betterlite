use betterlite_openai::threads::operations::ThreadReply;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_assistant_dispatch::call::{
    OpenAiAssistantOperation, ThreadsCall, ThreadsOperation,
};
use serde_json::{Value, json};

use super::input::{Input, ThreadsOp};
use super::run::options;

// The Python side always calls the async helpers, so every thread call asks for the async client.
pub(super) fn threads(
    input: &Input,
    operation: ThreadsOp,
) -> Result<OpenAiAssistantOperation, Box<HelperError>> {
    let thread_id = input.thread_id.clone();
    let thread = match operation {
        ThreadsOp::CreateThread => {
            ThreadsOperation::Create(serde_json::from_value(input.data.clone())?)
        }
        ThreadsOp::RetrieveThread => ThreadsOperation::Retrieve(thread_id),
        ThreadsOp::AddMessage => ThreadsOperation::AddMessage(
            thread_id,
            Box::new(serde_json::from_value(input.data.clone())?),
        ),
        ThreadsOp::ListMessages => ThreadsOperation::ListMessages(thread_id),
    };
    Ok(OpenAiAssistantOperation::Threads(ThreadsCall {
        options: options(input),
        async_requested: true,
        operation: thread,
    }))
}

pub(super) fn thread_view(reply: &ThreadReply) -> Value {
    match reply {
        ThreadReply::Thread(thread) => json!({"kind": "thread", "id": thread.id}),
        ThreadReply::Message(message) => {
            json!({"kind": "message", "id": message.id, "status": message.status})
        }
        ThreadReply::Page(page) => {
            let ids: Vec<&str> = page.data.iter().map(|item| item.id.as_str()).collect();
            json!({"kind": "page", "ids": ids})
        }
    }
}
