use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::records::{ThreadMessage, ThreadMessagePage};

use super::connection::{MessageParams, PrepareInput, prepare};
use super::error::{MessageError, MessageFailure, failure};
use super::options::MessageData;
use super::provider::{self, MessageContext, ProviderCall, ProviderReply};

// add_message's unsupported-provider message names create_thread, not add_message (main.py#L1000-L1008)
const APPEND_OPERATION: &str = "create_thread";
const LIST_OPERATION: &str = "get_messages";

// add_message's and get_messages's positional arguments plus the keyword arguments they read
pub struct MessageRequest<'request> {
    pub provider: &'request str,
    pub thread_id: &'request str,
    // add_message builds one from role, content, attachments and metadata; get_messages passes none
    pub data: Option<&'request MessageData>,
    pub params: &'request MessageParams,
    pub drop_params: bool,
}

pub enum Reply {
    Message(Box<ThreadMessage>),
    Page(Box<ThreadMessagePage>),
}

// litellm.add_message: the provider exception reaches the caller unchanged (main.py#L892-L1011)
pub async fn add_message<T>(
    request: &MessageRequest<'_>,
    context: &MessageContext<'_, T>,
) -> Result<Reply, MessageError>
where
    T: AssistantsTransport,
{
    append(request, context, false).await
}

// litellm.a_add_message: the flag picks the async client and exception_type maps the failure (main.py#L841-L889)
pub async fn a_add_message<T>(
    request: &MessageRequest<'_>,
    context: &MessageContext<'_, T>,
) -> Result<Reply, MessageFailure>
where
    T: AssistantsTransport,
{
    append(request, context, true)
        .await
        .map_err(|error| failure(error, request.provider, context.redactor))
}

// litellm.get_messages: the provider exception reaches the caller unchanged (main.py#L1057-L1161)
pub async fn get_messages<T>(
    request: &MessageRequest<'_>,
    context: &MessageContext<'_, T>,
) -> Result<Reply, MessageError>
where
    T: AssistantsTransport,
{
    list(request, context, false).await
}

// litellm.aget_messages: the flag picks the async client and exception_type maps the failure (main.py#L1014-L1054)
pub async fn aget_messages<T>(
    request: &MessageRequest<'_>,
    context: &MessageContext<'_, T>,
) -> Result<Reply, MessageFailure>
where
    T: AssistantsTransport,
{
    list(request, context, true)
        .await
        .map_err(|error| failure(error, request.provider, context.redactor))
}

async fn append<T>(
    request: &MessageRequest<'_>,
    context: &MessageContext<'_, T>,
    asynchronous: bool,
) -> Result<Reply, MessageError>
where
    T: AssistantsTransport,
{
    let prepared = prepare(&PrepareInput {
        provider: request.provider,
        operation: APPEND_OPERATION,
        data: request.data,
        params: request.params,
        settings: context.settings,
        drop_params: request.drop_params,
        asynchronous,
    })?;
    let data = prepared
        .data
        .as_ref()
        .ok_or(MessageError::UnexpectedReply {
            operation: APPEND_OPERATION,
        })?;
    match provider::send(
        &prepared,
        context,
        ProviderCall::Append {
            thread_id: request.thread_id,
            data,
        },
    )
    .await?
    {
        ProviderReply::Message(message) => Ok(Reply::Message(message)),
        ProviderReply::Page(_) => Err(MessageError::UnexpectedReply {
            operation: APPEND_OPERATION,
        }),
    }
}

async fn list<T>(
    request: &MessageRequest<'_>,
    context: &MessageContext<'_, T>,
    asynchronous: bool,
) -> Result<Reply, MessageError>
where
    T: AssistantsTransport,
{
    let prepared = prepare(&PrepareInput {
        provider: request.provider,
        operation: LIST_OPERATION,
        data: request.data,
        params: request.params,
        settings: context.settings,
        drop_params: request.drop_params,
        asynchronous,
    })?;
    match provider::send(&prepared, context, ProviderCall::List(request.thread_id)).await? {
        ProviderReply::Page(page) => Ok(Reply::Page(page)),
        ProviderReply::Message(_) => Err(MessageError::UnexpectedReply {
            operation: LIST_OPERATION,
        }),
    }
}
