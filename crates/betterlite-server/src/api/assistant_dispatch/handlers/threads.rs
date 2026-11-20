use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_core::api::assistant_threads::{
    AssistantThreads, CreateThreadCall, GetThreadCall, ThreadFacade as _,
};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::threads::records::ThreadMessageData;

use super::super::AssistantDispatcher;
use super::super::Merged;
use super::super::error::AssistantDispatchError;
use super::super::outcome::AssistantOutcome;
use super::super::request::AssistantRequest;
use super::support::{optional, required_text};
use super::{Deps, Dispatched, Sources};

// both wrappers hand the facade the same borrowed settings, credentials and dedicated session
fn facade<'a, T: Deps, S: Sources>(
    dispatcher: &'a AssistantDispatcher<'_, '_, T, S>,
    resources: &'a ResourceSdkSettings,
) -> AssistantThreads<'a, T> {
    AssistantThreads {
        transport: Arc::clone(&dispatcher.transport),
        resources,
        azure: dispatcher.azure,
        environment: dispatcher.environment,
        sources: &dispatcher.sources,
        redactor: dispatcher.redactor,
        refresh_enabled: false,
        session: SessionKind::Dedicated,
    }
}

pub(super) async fn create<T: Deps, S: Sources>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: Merged,
    request: &AssistantRequest<'_>,
) -> Dispatched<T> {
    let mut kwargs = merged.kwargs;
    let messages = optional::<Vec<ThreadMessageData>>(&kwargs, "messages")?;
    let metadata = optional::<BTreeMap<String, Json>>(&kwargs, "metadata")?;
    // accepted for signature parity and then never forwarded, on either provider
    let tool_resources = kwargs
        .get("tool_resources")
        .filter(|value| !matches!(value, Json::Null))
        .cloned();
    for name in ["messages", "metadata", "tool_resources"] {
        kwargs.remove(name);
    }
    let resources = ResourceSdkSettings::from(dispatcher.resources);
    let threads = facade(dispatcher, &resources);
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
        Err(failure) => Err(AssistantDispatchError::Thread {
            failure: Box::new(failure),
        }),
    }
}

pub(super) async fn get<T: Deps, S: Sources>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: Merged,
    request: &AssistantRequest<'_>,
) -> Dispatched<T> {
    let thread_id = required_text(&merged.kwargs, "thread_id", "aget_thread")?;
    let mut kwargs = merged.kwargs;
    kwargs.remove("thread_id");
    let resources = ResourceSdkSettings::from(dispatcher.resources);
    let threads = facade(dispatcher, &resources);
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
        Err(failure) => Err(AssistantDispatchError::Thread {
            failure: Box::new(failure),
        }),
    }
}
