use std::sync::Arc;

use betterlite_core::api::assistant_read_delete::request::{
    AssistantOperation as ReadDeleteOperation, AssistantReadDeleteInput, AssistantWrapper,
};
use betterlite_core::api::assistant_read_delete::sdk_dispatch::SdkDispatch;
use betterlite_core::api::assistant_read_delete::{AssistantReadDelete, AssistantStep};
use betterlite_core::api::assistant_records::AssistantCreateInput;
use betterlite_core::api::assistant_records::service::AssistantRecords;
use betterlite_models::decisions::Json;

use super::super::AssistantDispatcher;
use super::super::Merged;
use super::super::client::AssistantClient;
use super::super::error::{AssistantDispatchError, AssistantFailure};
use super::super::outcome::AssistantOutcome;
use super::super::request::AssistantRequest;
use super::support::{optional, parse_value, subset, text};
use super::{Deps, Dispatched, Sources};

// the kwargs GenericLiteLLMParams and get_litellm_params read for get/delete
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

pub(super) async fn create<T, S>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: Merged,
    request: &AssistantRequest<'_>,
) -> Dispatched<T>
where
    T: Deps,
    S: Sources,
{
    let mut fields = merged.kwargs;
    fields.insert(String::from("provider"), Json::Text(merged.provider));
    let mut input: AssistantCreateInput = parse_value(&Json::Object(fields.into_iter().collect()))?;
    input.client = request
        .client
        .as_ref()
        .map(|client| client.class.kinds().client);
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
        Err(failure) => Err(AssistantDispatchError::Failure {
            failure: Box::new(AssistantFailure::from(*failure)),
        }),
    }
}

pub(super) async fn read_or_delete<T, S>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: &Merged,
    operation: ReadDeleteOperation,
    request: &AssistantRequest<'_>,
) -> Dispatched<T>
where
    T: Deps,
    S: Sources,
{
    let input = read_delete_input(merged, operation, request.client.as_ref())?;
    let facade = AssistantReadDelete {
        dispatch: SdkDispatch {
            transport: Arc::clone(&dispatcher.transport),
            sources: dispatcher.sources.clone(),
            environment: dispatcher.environment.clone(),
            azure: dispatcher.azure.clone(),
        },
        limits: dispatcher.limits,
        settings: dispatcher.resources.clone(),
    };
    match facade.run(&input).await {
        Ok(result) => step(result.step),
        Err(error) => Err(AssistantDispatchError::read_delete(error)),
    }
}

fn step<T>(step: AssistantStep) -> Dispatched<T> {
    match step {
        AssistantStep::Page(page) => Ok(AssistantOutcome::Assistants(page)),
        AssistantStep::Deleted(deleted) => Ok(AssistantOutcome::Deleted(deleted)),
        AssistantStep::Coroutine => Err(AssistantDispatchError::Invalid {
            message: String::from("the asynchronous wrapper returned a coroutine"),
        }),
    }
}

fn read_delete_input(
    merged: &Merged,
    operation: ReadDeleteOperation,
    client: Option<&AssistantClient>,
) -> Result<AssistantReadDeleteInput, AssistantDispatchError> {
    let assistant_id = match operation {
        ReadDeleteOperation::Delete => {
            Some(text(&merged.kwargs, "assistant_id")?.ok_or_else(|| {
                AssistantDispatchError::missing_arguments("delete_assistant", &["assistant_id"])
            })?)
        }
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
        extra_body: merged
            .kwargs
            .get("extra_body")
            .map(parse_value)
            .transpose()?,
        client: client.map(AssistantClient::read_delete),
        after: text(&merged.kwargs, "after")?,
        before: text(&merged.kwargs, "before")?,
        limit: optional(&merged.kwargs, "limit")?,
        order: text(&merged.kwargs, "order")?,
        params: parse_value(&Json::Object(
            subset(&merged.kwargs, &READ_DELETE_PARAMS)
                .into_iter()
                .collect(),
        ))?,
    })
}
