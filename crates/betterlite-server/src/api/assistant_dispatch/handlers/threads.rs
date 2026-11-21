use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_core::api::assistant_threads::error::ThreadFailure;
use betterlite_core::api::assistant_threads::{
    AssistantThreads, CreateThreadCall, GetThreadCall, SuppliedClient as ThreadClient,
    ThreadFacade as _, ThreadOperation,
};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::threads::records::{ThreadMessageData, ThreadRecord};

use super::super::AssistantDispatcher;
use super::super::Merged;
use super::super::error::AssistantDispatchError;
use super::super::outcome::AssistantOutcome;
use super::super::request::AssistantRequest;
use super::support::{optional, text};
use super::{Deps, Dispatched, Sources};

// what create_thread reads off the wrapper's kwargs before its body is built
struct ThreadData {
    messages: Option<Vec<ThreadMessageData>>,
    metadata: Option<BTreeMap<String, Json>>,
    tool_resources: Option<Json>,
}

pub(super) async fn call<T: Deps, S: Sources>(
    dispatcher: &AssistantDispatcher<'_, '_, T, S>,
    merged: Merged,
    request: &AssistantRequest<'_>,
    operation: ThreadOperation,
) -> Dispatched<T> {
    let mut kwargs = merged.kwargs;
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
    let result = match operation {
        ThreadOperation::Create => {
            let call = create_call(dispatcher, request, merged.provider, &mut kwargs)?;
            threads.acreate_thread(call).await
        }
        ThreadOperation::Get => {
            let id = thread_id(&mut kwargs)?;
            let call = GetThreadCall {
                custom_llm_provider: merged.provider,
                thread_id: id,
                client: supplied(dispatcher, request),
                kwargs: &mut kwargs,
            };
            threads.aget_thread(call).await
        }
    };
    outcome(result)
}

fn create_call<'call, T: Deps, S: Sources>(
    dispatcher: &'call AssistantDispatcher<'_, '_, T, S>,
    request: &'call AssistantRequest<'_>,
    provider: String,
    kwargs: &'call mut BTreeMap<String, Json>,
) -> Result<CreateThreadCall<'call, T>, AssistantDispatchError> {
    let data = thread_data(kwargs)?;
    Ok(CreateThreadCall {
        custom_llm_provider: provider,
        messages: data.messages,
        metadata: data.metadata,
        tool_resources: data.tool_resources,
        client: supplied(dispatcher, request),
        kwargs,
    })
}

// get_thread's own named argument: the route passes it beside the body
fn thread_id(kwargs: &mut BTreeMap<String, Json>) -> Result<String, AssistantDispatchError> {
    let thread_id = text(kwargs, "thread_id")?
        .ok_or_else(|| AssistantDispatchError::missing_arguments("aget_thread", &["thread_id"]))?;
    kwargs.remove("thread_id");
    Ok(thread_id)
}

// the client object the caller handed reaches either installed wrapper unchanged
fn supplied<'dispatcher, T: Deps, S: Sources>(
    dispatcher: &'dispatcher AssistantDispatcher<'_, '_, T, S>,
    request: &'dispatcher AssistantRequest<'_>,
) -> Option<ThreadClient<'dispatcher, T>> {
    request
        .client
        .as_ref()
        .map(|client| client.thread(Arc::clone(&dispatcher.transport)))
}

// create_thread's named arguments are parsed here and never forwarded to the SDK owner
fn thread_data(kwargs: &mut BTreeMap<String, Json>) -> Result<ThreadData, AssistantDispatchError> {
    let messages = optional::<Vec<ThreadMessageData>>(kwargs, "messages")?;
    let metadata = optional::<BTreeMap<String, Json>>(kwargs, "metadata")?;
    // create_thread's own argument: the SDK owner accepts it for signature parity and never forwards it
    let tool_resources = kwargs.get("tool_resources").cloned();
    for name in ["messages", "metadata", "tool_resources"] {
        kwargs.remove(name);
    }
    Ok(ThreadData {
        messages,
        metadata,
        tool_resources,
    })
}

// both installed wrappers project the same record into the dispatcher's answer
fn outcome<T>(result: Result<Option<ThreadRecord>, ThreadFailure>) -> Dispatched<T> {
    match result {
        Ok(thread) => Ok(AssistantOutcome::Thread(thread)),
        Err(failure) => Err(AssistantDispatchError::Thread {
            failure: Box::new(failure),
        }),
    }
}
