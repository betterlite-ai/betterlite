use std::sync::Arc;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use tokio::runtime::Handle;

use super::ResponsesStreamHooks;
use super::deployment::{call_type_name, resolve_call_type};
use super::ports::{POST_SUCCESS_TASK, ResponseMetadataCall};
use super::state::ResponsesStreamHookState;
use crate::api::responses_stream_primitives::state::ResponsesStreamBaseState;
use crate::api::responses_stream_primitives::tasks::spawn_logging_task;
use crate::routes::CallType;

impl ResponsesStreamHooks<'_> {
    // _run_post_success_hooks: update response metadata, then hand the terminal event to B5367's hooks
    pub async fn run_post_success_hooks(
        &self,
        state: &ResponsesStreamBaseState,
        callback: &mut ResponsesStreamHookState,
        details: &IndexMap<String, Json>,
        explicit_call_type: Option<&str>,
        end_time: DateTime<FixedOffset>,
    ) {
        let Some(completed) = state.completed_response.as_ref() else {
            return;
        };
        let payload = success_payload(&state.request_data, details);
        if let Some(metadata) = self.metadata.as_ref() {
            let rejected = metadata
                .update_response_metadata(ResponseMetadataCall {
                    result: completed,
                    model: &state.model,
                    kwargs: &payload,
                    start_time: state.start_time,
                    end_time,
                    include_overhead: false,
                })
                .err();
            if let Some(error) = rejected {
                tracing::debug!("the responses response metadata update failed: {error}");
            }
        }
        let Some(hook) = self.success_hook.as_ref().map(Arc::clone) else {
            return;
        };
        let call_type = resolve_call_type(explicit_call_type, None).unwrap_or(CallType::Responses);
        let call_type = call_type_name(call_type).unwrap_or_default();
        let Ok(response) = serde_json::to_value(completed).and_then(serde_json::from_value::<Json>)
        else {
            return;
        };
        if Handle::try_current().is_ok() {
            let task_payload = payload.clone();
            let task_response = response.clone();
            let task_call_type = call_type.clone();
            let work = async move {
                hook.success_deployment_hook(&task_payload, &task_response, &task_call_type)
                    .await
                    .map(|_kept| ())
            };
            if let Some(handle) = spawn_logging_task(&state.pending_tasks, POST_SUCCESS_TASK, work)
            {
                callback.pending_logging_tasks.push(handle);
            }
            return;
        }
        // without a loop LiteLLM runs the coroutine to completion and swallows its error
        if let Err(error) = hook
            .success_deployment_hook(&payload, &response, &call_type)
            .await
        {
            tracing::debug!("the responses post-call success hook failed: {error}");
        }
    }
}

fn success_payload(
    request_data: &IndexMap<String, Json>,
    details: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut payload = request_data.clone();
    payload.extend(
        details
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    if !payload.contains_key("litellm_params") {
        let params = details
            .get("litellm_params")
            .cloned()
            .unwrap_or(Json::Object(IndexMap::new()));
        payload.insert("litellm_params".to_owned(), params);
    }
    payload
}
