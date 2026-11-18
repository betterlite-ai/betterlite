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
    pub async fn run_post_success_hooks(
        &self,
        state: &mut ResponsesStreamBaseState,
        callback: &mut ResponsesStreamHookState,
        details: &IndexMap<String, Json>,
        end_time: DateTime<FixedOffset>,
    ) {
        let Some((payload, response)) = self.post_payload(state, details, end_time) else {
            return;
        };
        let Some(hook) = self.success_hook.as_ref().map(Arc::clone) else {
            return;
        };
        let call_type =
            resolve_call_type(state.call_type.as_deref(), None).unwrap_or(CallType::Responses);
        let call_type_text = call_type_name(call_type).unwrap_or_default();
        if Handle::try_current().is_ok() {
            let task_payload = payload.clone();
            let task_response = response.clone();
            let task_call_type = call_type_text.clone();
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
            .success_deployment_hook(&payload, &response, &call_type_text)
            .await
        {
            tracing::debug!("the responses post-call success hook failed: {error}");
        }
    }
    fn post_payload(
        &self,
        state: &mut ResponsesStreamBaseState,
        details: &IndexMap<String, Json>,
        end_time: DateTime<FixedOffset>,
    ) -> Option<(IndexMap<String, Json>, Json)> {
        let completed = state.completed_response.as_mut()?;
        let payload = success_payload(&state.request_data, details);
        if let Some(metadata) = self.metadata.as_ref() {
            let _result = metadata.update_response_metadata(ResponseMetadataCall {
                result: &mut *completed,
                model: &state.model,
                kwargs: &payload,
                start_time: state.start_time,
                end_time,
                include_overhead: false,
            });
        }
        let response = serde_json::to_value(completed)
            .and_then(serde_json::from_value)
            .ok()?;
        Some((payload, response))
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
    payload
        .entry("litellm_params".to_owned())
        .or_insert_with(|| Json::Object(IndexMap::new()));
    payload
}
