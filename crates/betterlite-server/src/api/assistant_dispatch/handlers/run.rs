use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_core::api::assistant_run_sdk::call::RunThreadCall;
use betterlite_core::api::assistant_run_sdk::provider::ProviderRuns;
use betterlite_core::api::assistant_run_sdk::service::AssistantRunSdk;
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_models::decisions::Json;

use super::super::AssistantDispatcher;
use super::super::Merged;
use super::super::client::AssistantClient;
use super::super::error::AssistantDispatchError;
use super::super::outcome::AssistantOutcome;
use super::super::request::AssistantRequest;
use super::support::{optional, parse_value, require, subset, text};
use super::{Deps, Dispatched, Sources};

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

pub(super) async fn thread<T: Deps, S: Sources>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: Merged,
    request: &AssistantRequest<'_>,
) -> Dispatched<T> {
    require(
        &merged.kwargs,
        "arun_thread",
        &["thread_id", "assistant_id"],
    )?;
    let call = thread_call(merged.kwargs, merged.provider, request)?;
    let resources = ResourceSdkSettings::from(dispatcher.resources);
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
        Err(error) => Err(AssistantDispatchError::Run(Box::new(error))),
    }
}

// arun_thread's named arguments, the supplied client and the connection parameters left in kwargs
fn thread_call(
    kwargs: BTreeMap<String, Json>,
    provider: String,
    request: &AssistantRequest<'_>,
) -> Result<RunThreadCall, AssistantDispatchError> {
    let thread_id = text(&kwargs, "thread_id")?
        .ok_or_else(|| AssistantDispatchError::missing_arguments("arun_thread", &["thread_id"]))?;
    let assistant_id = text(&kwargs, "assistant_id")?.ok_or_else(|| {
        AssistantDispatchError::missing_arguments("arun_thread", &["assistant_id"])
    })?;
    Ok(RunThreadCall {
        provider,
        thread_id,
        assistant_id,
        additional_instructions: text(&kwargs, "additional_instructions")?,
        instructions: text(&kwargs, "instructions")?,
        metadata: optional(&kwargs, "metadata")?,
        model: text(&kwargs, "model")?,
        stream: kwargs.get("stream").map(parse_value).transpose()?,
        tools: optional(&kwargs, "tools")?,
        client: request.client.as_ref().map(AssistantClient::run),
        event_handler: request.event_handler.clone(),
        arun_thread: None,
        params: parse_value(&Json::Object(subset(&kwargs, &RUN_PARAMS)))?,
    })
}
