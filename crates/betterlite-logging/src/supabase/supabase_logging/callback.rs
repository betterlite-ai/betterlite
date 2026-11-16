use std::borrow::Cow;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;
use uuid::Uuid;

use super::rows::{InitiatedRow, RequestRow};
use super::{DEFAULT_END_USER, INITIATED_STATUS, SupabaseLogger};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::http_analytics_loggers::delivery::Delivery;
use crate::logging_pre_post::error::PrePostError;
use crate::logging_pre_post::hooks::{InputEventSink, PreCallEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failure,
}

pub struct SupabaseCallback<D: Delivery> {
    pub logger: Arc<SupabaseLogger<D>>,
    pub tasks: TaskTracker,
}

impl<D: Delivery + 'static> SupabaseCallback<D> {
    fn spawn(&self, event: &LoggerEvent, outcome: Outcome) {
        let payload = Arc::clone(&event.payload);
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tasks.spawn(async move {
            deliver_row(&logger, &payload, outcome).await;
        });
    }
}

#[async_trait]
impl<D: Delivery + 'static> CallbackLogger for SupabaseCallback<D> {
    fn callback_name(&self) -> &'static str {
        "supabase"
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.spawn(event, Outcome::Success);
        Ok(())
    }

    fn log_failure_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.spawn(event, Outcome::Failure);
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        deliver_row(&self.logger, &event.payload, Outcome::Success).await;
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        deliver_row(&self.logger, &event.payload, Outcome::Failure).await;
        Ok(())
    }
}

impl<D: Delivery + 'static> InputEventSink for SupabaseCallback<D> {
    fn log_input_event(&self, event: &PreCallEvent<'_>) -> Result<(), PrePostError> {
        let Some(row) = initiated_row(event) else {
            tracing::warn!("the supabase pre-call event carries no call id");
            return Ok(());
        };
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tasks.spawn(async move {
            if let Err(error) = logger.input_log_event(&row).await {
                tracing::warn!(%error, "the supabase initiated row could not be written");
            }
        });
        Ok(())
    }
}

// log_event swallows its exceptions and prints the traceback, so a failed write never reaches the caller
async fn deliver_row<D>(
    logger: &SupabaseLogger<D>,
    payload: &StandardLoggingPayload,
    outcome: Outcome,
) where
    D: Delivery,
{
    let Some(call_id) = call_id(payload, outcome) else {
        tracing::warn!("the supabase failure event carries no call id");
        return;
    };
    let Some(response_obj) = response_object(payload, outcome) else {
        return;
    };
    // success defaults the optional_params user; failure keeps model_call_details' user or none
    let user = match payload.model_parameters.get("user") {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    };
    let end_user = match outcome {
        Outcome::Success => Some(user.unwrap_or(DEFAULT_END_USER)),
        Outcome::Failure => user,
    };
    let row = RequestRow {
        messages: payload.messages.as_ref(),
        end_user,
        response_obj: response_obj.as_ref(),
        start_time: payload.start_time,
        end_time: payload.end_time,
        litellm_call_id: &call_id,
        total_cost: payload.response_cost,
    };
    if let Err(error) = logger.log_event(&row).await {
        tracing::warn!(%error, "the supabase request row could not be written");
    }
}

// LiteLLM hands log_event None as the result at the failure dispatch, so this object stands in for it
fn response_object(payload: &StandardLoggingPayload, outcome: Outcome) -> Option<Cow<'_, Json>> {
    match outcome {
        Outcome::Success => {
            // completion_cost raises for a model with no price entry, and log_event's catch then writes no row
            if payload.cost_failure_info.is_some() {
                tracing::warn!(
                    "the supabase success row is skipped, its cost could not be calculated"
                );
                return None;
            }
            Some(Cow::Borrowed(payload.response.as_ref()?))
        }
        Outcome::Failure => {
            let error = payload.error_str.as_ref()?;
            Some(Cow::Owned(Json::Object(IndexMap::from([
                ("model".to_owned(), Json::Text(payload.model.clone())),
                ("error".to_owned(), Json::Text(error.clone())),
            ]))))
        }
    }
}

// the success dispatch site reads litellm_params["litellm_call_id"] or a fresh uuid4, the failure site requires the key
fn call_id(payload: &StandardLoggingPayload, outcome: Outcome) -> Option<String> {
    match (payload.litellm_call_id.clone(), outcome) {
        (Some(call_id), _) => Some(call_id),
        (None, Outcome::Success) => Some(Uuid::new_v4().to_string()),
        (None, Outcome::Failure) => None,
    }
}

fn initiated_row(event: &PreCallEvent<'_>) -> Option<InitiatedRow> {
    let litellm_call_id = detail_text(event.details, "litellm_call_id")?.to_owned();
    Some(InitiatedRow {
        model: event.model.to_owned(),
        messages: event.messages.clone(),
        end_user: detail(event.details, "user")
            .cloned()
            .unwrap_or_else(|| Json::Text(DEFAULT_END_USER.to_owned())),
        status: INITIATED_STATUS,
        litellm_call_id,
    })
}

fn detail<'details>(details: &'details Json, name: &str) -> Option<&'details Json> {
    let Json::Object(fields) = details else {
        return None;
    };
    fields.get(name)
}

fn detail_text<'details>(details: &'details Json, name: &str) -> Option<&'details str> {
    match detail(details, name)? {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
