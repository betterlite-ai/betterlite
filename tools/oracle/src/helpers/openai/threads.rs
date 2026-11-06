use crate::helpers::{HelperError, HelperResult};
use betterlite_openai::assistants::client::{ClientKind, ClientOptions};
use betterlite_openai::assistants::operations::AssistantsApi;
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::operations::{ClientSetup, ThreadCall, ThreadReply, ThreadsApi};
use betterlite_openai::threads::records::{
    MessageContent, MessageRole, ThreadCreate, ThreadMessageData,
};
use input::{DispatchFields, OptionFields, Reply, ThreadsCall};
use secrecy::SecretString;
use serde_json::{Value, json};
use std::future::Future;
use std::sync::{Arc, PoisonError};
use std::time::Duration;
use wire::{MESSAGE, PAGE, THREAD, Wire, dispatch_wire, replying};

const THREAD_ID: &str = "thread_1";

fn fault(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        message,
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}

fn client_options(fields: &OptionFields) -> Result<ClientOptions, Box<HelperError>> {
    let timeout = match fields.timeout {
        Some(seconds) => {
            Some(Duration::try_from_secs_f64(seconds).map_err(|error| fault(error.to_string()))?)
        }
        None => None,
    };
    Ok(ClientOptions {
        api_key: fields.api_key.clone().map(SecretString::from),
        api_base: fields.api_base.clone(),
        timeout,
        max_retries: fields.max_retries,
        organization: fields.organization.clone(),
    })
}

// LiteLLM reports a failure as a raised exception, so the Rust side answers with the same stable field set.
fn failure(error: &ThreadsError) -> Result<Value, Box<HelperError>> {
    match error {
        ThreadsError::Status { status, .. } => Ok(json!({"kind": "status", "status": status})),
        ThreadsError::MissingApiKey => Ok(json!({"kind": "configuration"})),
        ThreadsError::EmptyThreadId | ThreadsError::DotSegment { .. } => {
            Ok(json!({"kind": "invalid_thread_id"}))
        }
        ThreadsError::BaseUrl { .. }
        | ThreadsError::Header { .. }
        | ThreadsError::Encode { .. }
        | ThreadsError::Transport { .. }
        | ThreadsError::Reply { .. } => Err(fault(error.to_string())),
    }
}

fn reply_value(reply: ThreadReply) -> Value {
    match reply {
        ThreadReply::Thread(thread) => serde_json::to_value(thread).unwrap_or(Value::Null),
        ThreadReply::Message(message) => serde_json::to_value(message).unwrap_or(Value::Null),
        ThreadReply::Page(page) => serde_json::to_value(page).unwrap_or(Value::Null),
    }
}

fn answer(wire: &Wire, result: Result<Value, ThreadsError>) -> HelperResult {
    let requests = wire
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let body = match result {
        Ok(value) => json!({"requests": requests, "result": value}),
        Err(error) => json!({"requests": requests, "error": failure(&error)?}),
    };
    Ok(body)
}

fn block_on<T, F>(future: F) -> Result<T, Box<HelperError>>
where
    F: Future<Output = T>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| fault(error.to_string()))?;
    Ok(runtime.block_on(future))
}

const fn client_class(kind: ClientKind) -> &'static str {
    match kind {
        ClientKind::Sync => "OpenAI",
        ClientKind::Async => "AsyncOpenAI",
    }
}

fn exchange(
    wire: &Arc<Wire>,
    fields: &OptionFields,
    flag: bool,
    request: ThreadCall<'_>,
) -> Result<Result<Value, ThreadsError>, Box<HelperError>> {
    let api = ThreadsApi {
        assistants: AssistantsApi {
            transport: Arc::clone(wire),
        },
    };
    let options = client_options(fields)?;
    let setup = ClientSetup {
        options,
        supplied: None,
        async_requested: flag,
    };
    block_on(api.call(setup, request)).map(|reply| reply.map(reply_value))
}

fn run_one(
    given: Option<&Reply>,
    canned: &str,
    fields: &OptionFields,
    flag: bool,
    request: ThreadCall<'_>,
) -> HelperResult {
    let wire = replying(given, canned);
    let outcome = exchange(&wire, fields, flag, request)?;
    answer(&wire, outcome)
}

fn dispatch_kinds(
    api: &ThreadsApi<Wire>,
    options: &ClientOptions,
    fields: &DispatchFields,
) -> Vec<&'static str> {
    let flags = [
        fields.acreate_thread.unwrap_or(false),
        fields.aget_thread.unwrap_or(false),
        fields.a_add_message.unwrap_or(false),
        fields.aget_messages.unwrap_or(false),
    ];
    flags
        .into_iter()
        .map(|flag| {
            let setup = ClientSetup {
                options: options.clone(),
                supplied: None,
                async_requested: flag,
            };
            client_class(api.client(setup).kind)
        })
        .collect()
}

// The four operations run in LiteLLM's order, each carrying the flag its dispatch function reads.
fn dispatch_calls<'request>(
    fields: &DispatchFields,
    create: &'request ThreadCreate,
    data: &'request ThreadMessageData,
) -> [(bool, ThreadCall<'request>); 4] {
    [
        (
            fields.acreate_thread.unwrap_or(false),
            ThreadCall::Create(create),
        ),
        (
            fields.aget_thread.unwrap_or(false),
            ThreadCall::Retrieve(THREAD_ID),
        ),
        (
            fields.a_add_message.unwrap_or(false),
            ThreadCall::AddMessage {
                thread_id: THREAD_ID,
                data,
            },
        ),
        (
            fields.aget_messages.unwrap_or(false),
            ThreadCall::ListMessages(THREAD_ID),
        ),
    ]
}

fn dispatch(fields: &DispatchFields) -> HelperResult {
    let wire = dispatch_wire(fields.reply.as_ref());
    let api = ThreadsApi {
        assistants: AssistantsApi {
            transport: Arc::clone(&wire),
        },
    };
    let create = ThreadCreate {
        messages: None,
        metadata: None,
    };
    let data = ThreadMessageData {
        role: MessageRole::User,
        content: MessageContent::Text(String::from("hi")),
        attachments: None,
        metadata: None,
    };
    let mut values = Vec::new();
    for (flag, request) in dispatch_calls(fields, &create, &data) {
        values.push(exchange(&wire, &fields.options, flag, request)?);
    }
    let result = match values
        .into_iter()
        .collect::<Result<Vec<Value>, ThreadsError>>()
    {
        Ok(collected) => Value::from(collected),
        Err(error) => return answer(&wire, Err(error)),
    };
    let options = client_options(&fields.options)?;
    let kinds = dispatch_kinds(&api, &options, fields);
    let requests = wire
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(json!({"requests": requests, "results": result, "kinds": kinds}))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = ThreadsCall;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            ThreadsCall::CreateThread(fields) => {
                let data = ThreadCreate {
                    messages: fields.messages.clone(),
                    metadata: fields.metadata.clone(),
                };
                run_one(
                    fields.reply.as_ref(),
                    THREAD,
                    &fields.options,
                    fields.acreate_thread.unwrap_or(false),
                    ThreadCall::Create(&data),
                )
            }
            ThreadsCall::GetThread(fields) => run_one(
                fields.reply.as_ref(),
                THREAD,
                &fields.options,
                fields.aget_thread.unwrap_or(false),
                ThreadCall::Retrieve(fields.thread_id.as_str()),
            ),
            ThreadsCall::AddMessage(fields) => run_one(
                fields.reply.as_ref(),
                MESSAGE,
                &fields.options,
                fields.a_add_message.unwrap_or(false),
                ThreadCall::AddMessage {
                    thread_id: fields.thread_id.as_str(),
                    data: &fields.data,
                },
            ),
            ThreadsCall::GetMessages(fields) => run_one(
                fields.reply.as_ref(),
                PAGE,
                &fields.options,
                fields.aget_messages.unwrap_or(false),
                ThreadCall::ListMessages(fields.thread_id.as_str()),
            ),
            ThreadsCall::Dispatch(fields) => dispatch(fields),
        }
    }
}

mod input;
mod tests;
mod wire;
