use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::records::{ThreadMessage, ThreadMessagePage};

use super::connection::{MessageParams, PrepareInput, prepare};
use super::error::{MessageError, MessageFailure, failure};
use super::options::MessageData;
use super::provider::{self, MessageContext, ProviderCall};

// add_message's unsupported-provider message names create_thread, not add_message (main.py#L1000-L1008)
const APPEND_OPERATION: &str = "create_thread";
const LIST_OPERATION: &str = "get_messages";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Append,
    List,
}

// add_message's and get_messages's positional arguments plus the keyword arguments they read
pub struct MessageRequest<'request> {
    pub provider: &'request str,
    pub thread_id: &'request str,
    // add_message builds one from role, content, attachments and metadata; get_messages passes none
    pub data: Option<&'request MessageData>,
    pub params: &'request MessageParams,
    pub drop_params: bool,
}

#[derive(Debug)]
pub enum Reply {
    Message(Box<ThreadMessage>),
    Page(Box<ThreadMessagePage>),
}

// the operation picks the wrapper and the mode the provider client (main.py#L841-L1161)
pub async fn message_call<T>(
    request: &MessageRequest<'_>,
    context: &MessageContext<'_, T>,
    operation: Operation,
    asynchronous: bool,
) -> Result<Reply, MessageFailure>
where
    T: AssistantsTransport,
{
    async {
        let prepared = prepare(&PrepareInput {
            provider: request.provider,
            operation: operation_name(operation),
            data: request.data,
            params: request.params,
            settings: context.settings,
            drop_params: request.drop_params,
            asynchronous,
        })?;
        let data = prepared.data.as_ref();
        let call = match operation {
            Operation::Append => ProviderCall::Append {
                thread_id: request.thread_id,
                data: data.ok_or(MessageError::UnexpectedReply {
                    operation: APPEND_OPERATION,
                })?,
            },
            Operation::List => ProviderCall::List(request.thread_id),
        };
        match (operation, provider::send(&prepared, context, call).await?) {
            (Operation::Append, Reply::Message(message)) => Ok(Reply::Message(message)),
            (Operation::List, Reply::Page(page)) => Ok(Reply::Page(page)),
            (wrong, _) => Err(MessageError::UnexpectedReply {
                operation: operation_name(wrong),
            }),
        }
    }
    .await
    .map_err(|error| failure(error, request.provider, context.redactor))
}

const fn operation_name(operation: Operation) -> &'static str {
    match operation {
        Operation::Append => APPEND_OPERATION,
        Operation::List => LIST_OPERATION,
    }
}
