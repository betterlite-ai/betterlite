use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::credentials::source::CredentialSources;
use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::assistant_messages::connection::MessageParams;
use betterlite_core::api::assistant_messages::options::MessageData;
use betterlite_core::api::assistant_messages::provider::MessageContext;
use betterlite_core::api::assistant_messages::service::{
    MessageRequest, Operation as MessageOperation, Reply, message_call,
};
use betterlite_core::api::assistant_read_delete::request::{
    AssistantOperation as ReadDeleteOperation, AssistantParams, AssistantReadDeleteInput,
    AssistantWrapper,
};
use betterlite_core::api::assistant_read_delete::sdk_dispatch::SdkDispatch;
use betterlite_core::api::assistant_read_delete::{AssistantReadDelete, AssistantStep};
use betterlite_core::api::assistant_records::AssistantCreateInput;
use betterlite_core::api::assistant_records::service::AssistantRecords;
use betterlite_core::api::assistant_run_sdk::call::{RunFlag, RunParams, RunThreadCall};
use betterlite_core::api::assistant_run_sdk::provider::ProviderRuns;
use betterlite_core::api::assistant_run_sdk::service::AssistantRunSdk;
use betterlite_core::api::assistant_threads::{
    AssistantThreads, CreateThreadCall, GetThreadCall, ThreadFacade as _,
};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::stream::{RunStream, RunStreamTransport};
use betterlite_openai::threads::records::ThreadMessageData;
use serde::de::DeserializeOwned;

use super::AssistantDispatcher;
use super::Merged;
use super::client::AssistantClient;
use super::error::{AssistantDispatchError, AssistantFailure};
use super::operation::AssistantOperation;
use super::outcome::AssistantOutcome;
use super::request::AssistantRequest;

const READ_DELETE_PARAMS: [&str; 14] = [
    "api_key",
    "api_base",
    "api_version",
    "organization",
    "max_retries",
    "timeout",
    "request_timeout",
    "azure_ad_token",
    "tenant_id",
    "client_id",
    "client_secret",
    "azure_username",
    "azure_password",
    "azure_scope",
];

const RUN_PARAMS: [&str; 15] = [
    "api_base",
    "api_key",
    "api_version",
    "organization",
    "azure_ad_token",
    "tenant_id",
    "client_id",
    "client_secret",
    "azure_username",
    "azure_password",
    "azure_scope",
    "timeout",
    "request_timeout",
    "max_retries",
    "extra_body",
];

pub(super) async fn run<T, S>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    request: &AssistantRequest<'_>,
    merged: Merged,
) -> Result<AssistantOutcome<RunStream<T>>, AssistantDispatchError>
where
    T: AssistantsTransport + RunStreamTransport,
    S: CredentialSources + Clone + Send + Sync,
{
    match request.operation {
        AssistantOperation::CreateAssistant => create(dispatcher, merged, request).await,
        AssistantOperation::ListAssistants => {
            list_or_delete(dispatcher, merged, ReadDeleteOperation::List, request).await
        }
        AssistantOperation::DeleteAssistant => {
            list_or_delete(dispatcher, merged, ReadDeleteOperation::Delete, request).await
        }
        AssistantOperation::CreateThread => create_thread(dispatcher, merged, request).await,
        AssistantOperation::GetThread => get_thread(dispatcher, merged, request).await,
        AssistantOperation::CreateMessage => message(dispatcher, merged, request, true).await,
        AssistantOperation::ListMessages => message(dispatcher, merged, request, false).await,
        AssistantOperation::RunThread => run_thread(dispatcher, merged, request).await,
    }
}

async fn create<T, S>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: Merged,
    request: &AssistantRequest<'_>,
) -> Result<AssistantOutcome<RunStream<T>>, AssistantDispatchError>
where
    T: AssistantsTransport + RunStreamTransport,
    S: CredentialSources + Clone + Send + Sync,
{
    let mut fields = merged.kwargs;
    fields.insert(String::from("provider"), Json::Text(merged.provider));
    let mut input: AssistantCreateInput = parse(&fields)?;
    input.client = request.client.as_ref().map(AssistantClient::create_class);
    let records = AssistantRecords {
        settings: dispatcher.resources.clone(),
        azure: dispatcher.azure.clone(),
        environment: dispatcher.environment,
        sources: &dispatcher.sources,
        resolution: dispatcher.resolution,
        redactor: dispatcher.redactor,
        transport: Arc::clone(&dispatcher.transport),
    };
    match records.acreate_assistants(&input).await {
        Ok(created) => Ok(AssistantOutcome::Assistant(Box::new(created.assistant))),
        Err(failure) => Err(AssistantDispatchError::Failure(AssistantFailure::from(
            *failure,
        ))),
    }
}

async fn list_or_delete<T, S>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: Merged,
    operation: ReadDeleteOperation,
    request: &AssistantRequest<'_>,
) -> Result<AssistantOutcome<RunStream<T>>, AssistantDispatchError>
where
    T: AssistantsTransport + RunStreamTransport,
    S: CredentialSources + Clone + Send + Sync,
{
    let input = read_delete_input(&merged, operation, request.client.as_ref())?;
    let facade = AssistantReadDelete {
        dispatch: SdkDispatch {
            transport: Arc::clone(&dispatcher.transport),
            sources: dispatcher.sources.clone(),
            environment: dispatcher.environment.clone(),
            azure: dispatcher.azure.clone(),
        },
        limits: dispatcher.limits.clone(),
        settings: dispatcher.resources.clone(),
    };
    match facade.run(&input).await {
        Ok(result) => match result.step {
            AssistantStep::Page(page) => Ok(AssistantOutcome::Assistants(page)),
            AssistantStep::Deleted(deleted) => Ok(AssistantOutcome::Deleted(deleted)),
            AssistantStep::Coroutine => Err(AssistantDispatchError::invalid_message(String::from(
                "the asynchronous wrapper returned a coroutine",
            ))),
        },
        Err(error) => Err(AssistantDispatchError::read_delete(error)),
    }
}

fn read_delete_input(
    merged: &Merged,
    operation: ReadDeleteOperation,
    client: Option<&AssistantClient>,
) -> Result<AssistantReadDeleteInput, AssistantDispatchError> {
    let assistant_id = match operation {
        ReadDeleteOperation::Delete => Some(required_text(
            &merged.kwargs,
            "assistant_id",
            "delete_assistant",
        )?),
        ReadDeleteOperation::List => None,
    };
    Ok(AssistantReadDeleteInput {
        operation,
        provider: merged.provider.clone(),
        assistant_id,
        entry: AssistantWrapper::Async,
        aget_assistants: optional(&merged.kwargs, "aget_assistants")?,
        async_delete_assistants: optional(&merged.kwargs, "async_delete_assistants")?,
        // the input's double option tells an omitted key from an explicit null
        extra_body: match merged.kwargs.get("extra_body") {
            None => None,
            Some(value) => Some(parse_value(value)?),
        },
        client: client.map(AssistantClient::read_delete),
        after: text(&merged.kwargs, "after")?,
        before: text(&merged.kwargs, "before")?,
        limit: optional(&merged.kwargs, "limit")?,
        order: text(&merged.kwargs, "order")?,
        params: parse(&subset(&merged.kwargs, &READ_DELETE_PARAMS))?,
    })
}

async fn create_thread<T, S>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: Merged,
    request: &AssistantRequest<'_>,
) -> Result<AssistantOutcome<RunStream<T>>, AssistantDispatchError>
where
    T: AssistantsTransport + RunStreamTransport,
    S: CredentialSources + Clone + Send + Sync,
{
    let mut kwargs = merged.kwargs;
    let messages = optional::<Vec<ThreadMessageData>>(&kwargs, "messages")?;
    let metadata = optional::<BTreeMap<String, Json>>(&kwargs, "metadata")?;
    let tool_resources = kwargs
        .get("tool_resources")
        .filter(|value| !matches!(value, Json::Null))
        .cloned();
    kwargs.remove("messages");
    kwargs.remove("metadata");
    kwargs.remove("tool_resources");
    let resources = ResourceSdkSettings::from(dispatcher.resources);
    let threads = AssistantThreads {
        transport: Arc::clone(&dispatcher.transport),
        resources: &resources,
        azure: dispatcher.azure,
        environment: dispatcher.environment,
        sources: &dispatcher.sources,
        redactor: dispatcher.redactor,
        refresh_enabled: false,
        session: SessionKind::Dedicated,
    };
    let call = CreateThreadCall {
        custom_llm_provider: merged.provider,
        messages,
        metadata,
        tool_resources,
        client: request
            .client
            .as_ref()
            .map(|client| client.thread(Arc::clone(&dispatcher.transport))),
        kwargs: &mut kwargs,
    };
    match threads.acreate_thread(call).await {
        Ok(thread) => Ok(AssistantOutcome::Thread(thread)),
        Err(failure) => Err(AssistantDispatchError::Thread(Box::new(failure))),
    }
}

async fn get_thread<T, S>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: Merged,
    request: &AssistantRequest<'_>,
) -> Result<AssistantOutcome<RunStream<T>>, AssistantDispatchError>
where
    T: AssistantsTransport + RunStreamTransport,
    S: CredentialSources + Clone + Send + Sync,
{
    let thread_id = required_text(&merged.kwargs, "thread_id", "aget_thread")?;
    let mut kwargs = merged.kwargs;
    kwargs.remove("thread_id");
    let resources = ResourceSdkSettings::from(dispatcher.resources);
    let threads = AssistantThreads {
        transport: Arc::clone(&dispatcher.transport),
        resources: &resources,
        azure: dispatcher.azure,
        environment: dispatcher.environment,
        sources: &dispatcher.sources,
        redactor: dispatcher.redactor,
        refresh_enabled: false,
        session: SessionKind::Dedicated,
    };
    let call = GetThreadCall {
        custom_llm_provider: merged.provider,
        thread_id,
        client: request
            .client
            .as_ref()
            .map(|client| client.thread(Arc::clone(&dispatcher.transport))),
        kwargs: &mut kwargs,
    };
    match threads.aget_thread(call).await {
        Ok(thread) => Ok(AssistantOutcome::Thread(thread)),
        Err(failure) => Err(AssistantDispatchError::Thread(Box::new(failure))),
    }
}

async fn message<T, S>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: Merged,
    request: &AssistantRequest<'_>,
    append: bool,
) -> Result<AssistantOutcome<RunStream<T>>, AssistantDispatchError>
where
    T: AssistantsTransport + RunStreamTransport,
    S: CredentialSources + Clone + Send + Sync,
{
    let function = if append {
        "a_add_message"
    } else {
        "aget_messages"
    };
    let thread_id = required_text(&merged.kwargs, "thread_id", function)?;
    let mut kwargs = merged.kwargs;
    let data = if append {
        Some(message_data(&kwargs)?)
    } else {
        None
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
    let operation = if append {
        MessageOperation::Append
    } else {
        MessageOperation::List
    };
    let message_request = MessageRequest {
        provider: &merged.provider,
        thread_id: &thread_id,
        data: data.as_ref(),
        params: &params,
        drop_params: false,
    };
    match message_call(&message_request, &context, operation, true).await {
        Ok(Reply::Message(message)) => Ok(AssistantOutcome::Message(message)),
        Ok(Reply::Page(page)) => Ok(AssistantOutcome::Messages(page)),
        Err(failure) => Err(AssistantDispatchError::Message(failure)),
    }
}

// add_message builds its data from the named arguments; role and content have no default
fn message_data(kwargs: &BTreeMap<String, Json>) -> Result<MessageData, AssistantDispatchError> {
    required_present(kwargs, "role", "a_add_message")?;
    required_present(kwargs, "content", "a_add_message")?;
    let mut fields = BTreeMap::new();
    for name in ["role", "content", "attachments", "metadata"] {
        if let Some(value) = kwargs.get(name) {
            fields.insert(name.to_owned(), value.clone());
        }
    }
    parse(&fields)
}

async fn run_thread<T, S>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: Merged,
    request: &AssistantRequest<'_>,
) -> Result<AssistantOutcome<RunStream<T>>, AssistantDispatchError>
where
    T: AssistantsTransport + RunStreamTransport,
    S: CredentialSources + Clone + Send + Sync,
{
    let kwargs = &merged.kwargs;
    let thread_id = required_text(kwargs, "thread_id", "arun_thread")?;
    let assistant_id = required_text(kwargs, "assistant_id", "arun_thread")?;
    let resources = ResourceSdkSettings::from(dispatcher.resources);
    let call = RunThreadCall {
        provider: merged.provider,
        thread_id,
        assistant_id,
        additional_instructions: text(kwargs, "additional_instructions")?,
        instructions: text(kwargs, "instructions")?,
        metadata: nullable(kwargs, "metadata")?,
        model: text(kwargs, "model")?,
        stream: match kwargs.get("stream") {
            None => None,
            Some(value) => Some(parse_value(value)?),
        },
        tools: nullable(kwargs, "tools")?,
        client: request.client.as_ref().map(AssistantClient::run),
        event_handler: request.event_handler.clone(),
        arun_thread: None,
        params: parse(&subset(kwargs, &RUN_PARAMS))?,
    };
    let sdk = AssistantRunSdk {
        operations: ProviderRuns {
            transport: Arc::clone(&dispatcher.transport),
        },
        settings: &resources,
        azure: dispatcher.azure,
        environment: dispatcher.environment,
        sources: &dispatcher.sources,
        redactor: dispatcher.redactor,
    };
    match sdk.arun_thread(call).await {
        Ok(outcome) => Ok(AssistantOutcome::Run(outcome)),
        Err(error) => Err(AssistantDispatchError::Run(error)),
    }
}

fn text(
    kwargs: &BTreeMap<String, Json>,
    name: &str,
) -> Result<Option<String>, AssistantDispatchError> {
    match kwargs.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(AssistantDispatchError::invalid(name))
        }
    }
}

fn required_text(
    kwargs: &BTreeMap<String, Json>,
    name: &str,
    function: &str,
) -> Result<String, AssistantDispatchError> {
    text(kwargs, name)?.ok_or_else(|| AssistantDispatchError::missing_argument(function, name))
}

fn required_present(
    kwargs: &BTreeMap<String, Json>,
    name: &str,
    function: &str,
) -> Result<(), AssistantDispatchError> {
    if kwargs.contains_key(name) {
        Ok(())
    } else {
        Err(AssistantDispatchError::missing_argument(function, name))
    }
}

fn optional<T: DeserializeOwned>(
    kwargs: &BTreeMap<String, Json>,
    name: &str,
) -> Result<Option<T>, AssistantDispatchError> {
    match kwargs.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => parse_value(value).map(Some),
    }
}

// an untyped SDK value: a required JSON null reads as absent, a value is handed on as written
fn nullable(
    kwargs: &BTreeMap<String, Json>,
    name: &str,
) -> Result<Option<JsonValue>, AssistantDispatchError> {
    match kwargs.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => parse_value(value).map(Some),
    }
}

fn subset(kwargs: &BTreeMap<String, Json>, names: &[&str]) -> BTreeMap<String, Json> {
    names
        .iter()
        .filter_map(|name| {
            kwargs
                .get(*name)
                .map(|value| ((*name).to_owned(), value.clone()))
        })
        .collect()
}

fn parse<T: DeserializeOwned>(
    fields: &BTreeMap<String, Json>,
) -> Result<T, AssistantDispatchError> {
    let bytes = serde_json::to_vec(fields)
        .map_err(|error| AssistantDispatchError::invalid_message(error.to_string()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| AssistantDispatchError::invalid_message(error.to_string()))
}

fn parse_value<T: DeserializeOwned>(value: &Json) -> Result<T, AssistantDispatchError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| AssistantDispatchError::invalid_message(error.to_string()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| AssistantDispatchError::invalid_message(error.to_string()))
}
