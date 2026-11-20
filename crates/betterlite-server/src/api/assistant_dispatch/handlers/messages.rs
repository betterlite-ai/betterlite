use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_core::api::assistant_messages::connection::MessageParams;
use betterlite_core::api::assistant_messages::options::MessageData;
use betterlite_core::api::assistant_messages::provider::MessageContext;
use betterlite_core::api::assistant_messages::service::{
    MessageRequest, Operation as MessageOperation, Reply, message_call,
};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_models::decisions::Json;

use super::super::AssistantDispatcher;
use super::super::Merged;
use super::super::error::AssistantDispatchError;
use super::super::outcome::AssistantOutcome;
use super::support::{parse, require, required_text, subset};
use super::{Deps, Dispatched, Sources};

// add_message's named arguments; get_messages reads only the thread id
const NAMED: [&str; 4] = ["role", "content", "attachments", "metadata"];

pub(super) async fn call<T: Deps, S: Sources>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: Merged,
    operation: MessageOperation,
) -> Dispatched<T> {
    let function = function_name(operation);
    let names: &[&str] = match operation {
        MessageOperation::Append => &["thread_id", "role", "content"],
        MessageOperation::List => &["thread_id"],
    };
    require(&merged.kwargs, function, names)?;
    let thread_id = required_text(&merged.kwargs, "thread_id", function)?;
    let mut kwargs = merged.kwargs;
    let data = match operation {
        MessageOperation::Append => Some(message_data(&kwargs)?),
        MessageOperation::List => None,
    };
    for name in ["thread_id", "role", "content", "attachments", "metadata"] {
        kwargs.remove(name);
    }
    let params: MessageParams = parse(&kwargs)?;
    let resources = ResourceSdkSettings::from(dispatcher.resources);
    let context = MessageContext {
        settings: &resources,
        azure: dispatcher.azure,
        environment: dispatcher.environment,
        sources: &dispatcher.sources,
        transport: Arc::clone(&dispatcher.transport),
        redactor: dispatcher.redactor,
    };
    let message_request = build_request(&merged.provider, &thread_id, data.as_ref(), &params);
    match message_call(&message_request, &context, operation, true).await {
        Ok(Reply::Message(message)) => Ok(AssistantOutcome::Message(message)),
        Ok(Reply::Page(page)) => Ok(AssistantOutcome::Messages(page)),
        Err(failure) => Err(AssistantDispatchError::Message(failure)),
    }
}

// the installed function a missing argument names
const fn function_name(operation: MessageOperation) -> &'static str {
    match operation {
        MessageOperation::Append => "a_add_message",
        MessageOperation::List => "aget_messages",
    }
}

// add_message's data is built from the named arguments; get_messages builds none
fn message_data(kwargs: &BTreeMap<String, Json>) -> Result<MessageData, AssistantDispatchError> {
    parse(&subset(kwargs, &NAMED))
}

fn build_request<'request>(
    provider: &'request str,
    thread_id: &'request str,
    data: Option<&'request MessageData>,
    params: &'request MessageParams,
) -> MessageRequest<'request> {
    MessageRequest {
        provider,
        thread_id,
        data,
        params,
        drop_params: false,
    }
}
