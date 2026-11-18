use betterlite_core::api::responses_stream_hooks::cache::persist_cache;
use betterlite_core::api::responses_stream_hooks::deployment::chunk_hook;
use betterlite_core::api::responses_stream_hooks::failed::handle_logging_failed_response;
use betterlite_core::api::responses_stream_hooks::failed::maybe_raise_for_error_event;
use betterlite_core::api::responses_stream_hooks::failure::handle_failure;
use betterlite_core::api::responses_stream_hooks::logging::log_completed_response;
use betterlite_core::api::responses_stream_hooks::ports::Dispatch;
use betterlite_core::api::responses_stream_hooks::post_success::run_post_success_hooks;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::responses_stream_hooks::ResponsesStreamHooks;
use betterlite_core::api::responses_stream_hooks::deployment::ChunkHookContext;
use betterlite_core::api::responses_stream_hooks::events::nested_response;
use betterlite_core::api::responses_stream_hooks::failed::StreamFailure;
use betterlite_core::api::responses_stream_hooks::failure::await_pending_logging;
use betterlite_core::api::responses_stream_hooks::logging::logging_event;
use betterlite_core::api::responses_stream_hooks::state::ResponsesStreamHookState;
use betterlite_core::api::responses_stream_primitives::state::ResponsesStreamBaseState;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::Operation;
use super::fixtures::support::{Probe, Row, context, state};

struct Session<'context> {
    hooks: ResponsesStreamHooks<'context>,
    base: ResponsesStreamBaseState,
    callback: ResponsesStreamHookState,
    details: IndexMap<String, Json>,
}

pub(super) async fn run(operation: Operation, row: &Row) -> HelperResult {
    let clock = DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").or_value_error()?;
    let probe = Arc::new(Probe {
        row: row.clone(),
        records: Mutex::new(Vec::new()),
        clock,
    });
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let mut session = Session {
        hooks: context(&probe, &redactor),
        base: state(&probe),
        callback: ResponsesStreamHookState::default(),
        details: row.details.clone(),
    };
    let extra = session.invoke(operation, &probe).await?;
    await_pending_logging(&mut session.callback).await;
    session.base.pending_tasks.close();
    session.base.pending_tasks.wait().await;
    let records = probe
        .records
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(
        json!({"records": records, "cached": session.base.completed_response_cached,
        "logged": session.base.completed_response_logged,
        "failed": session.base.failure_handled, "extra": extra}),
    )
}

impl Session<'_> {
    async fn invoke(&mut self, operation: Operation, probe: &Probe) -> HelperResult {
        match operation {
            Operation::Cache => {
                persist_cache(&self.hooks, &mut self.base, Dispatch::Spawned).or_value_error()?;
                persist_cache(&self.hooks, &mut self.base, Dispatch::Spawned).or_value_error()?;
                Ok(Value::Null)
            }
            Operation::Logging => self.logging().await,
            Operation::Headers => self.headers(&probe.row),
            Operation::Failed => Ok(self.failed(probe).await),
            Operation::Chunk => Ok(self.chunk(&probe.row).await),
            Operation::Metadata => {
                run_post_success_hooks(
                    &self.hooks,
                    &mut self.base,
                    &mut self.callback,
                    &self.details,
                    probe.clock,
                )
                .await;
                Ok(Value::Null)
            }
            Operation::Failure => Ok(self.failure(probe).await),
        }
    }

    async fn failure(&mut self, probe: &Probe) -> Value {
        handle_raised_failure(&self.hooks, &mut self.base, &mut self.callback, probe).await;
        Value::Null
    }

    fn headers(&self, row: &Row) -> HelperResult {
        let copied = serde_json::to_value(logging_event(
            &row.event,
            &self.base.hidden_params,
            &self.base.raw_response_headers,
        ))?;
        Ok(copied
            .pointer("/response/_hidden_params")
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn logging(&mut self) -> HelperResult {
        log_completed_response(
            &self.hooks,
            &mut self.base,
            &mut self.callback,
            &self.details,
            Dispatch::Spawned,
        )
        .await
        .or_value_error()?;
        log_completed_response(
            &self.hooks,
            &mut self.base,
            &mut self.callback,
            &self.details,
            Dispatch::Spawned,
        )
        .await
        .or_value_error()?;
        Ok(json!({"deferred": self.callback.deferred_stream_complete_args.is_some()}))
    }

    async fn failed(&mut self, probe: &Probe) -> Value {
        handle_logging_failed_response(
            &self.hooks,
            &mut self.base,
            &mut self.callback,
            &mut self.details,
            Some(probe),
        )
        .await;
        maybe_raise_for_error_event(&self.hooks, &self.base, &probe.row.event)
            .err()
            .map_or(Value::Null, |error| failure_view(&error, &self.details))
    }

    async fn chunk(&mut self, row: &Row) -> Value {
        let hook_call = ChunkHookContext {
            explicit_call_type: row.call_type.as_deref(),
            logging_call_type: row.logging_type.as_deref(),
            request_data: Some(&row.request),
            model_call_details: &self.details,
        };
        let event = chunk_hook(
            &self.hooks,
            &mut self.callback,
            hook_call,
            row.event.clone(),
        )
        .await;
        json!({"id": nested_response(&event).map(|response| &response.id), "ran": self.callback.post_streaming_hooks_ran})
    }
}

async fn handle_raised_failure(
    hooks: &ResponsesStreamHooks<'_>,
    state: &mut ResponsesStreamBaseState,
    callback: &mut ResponsesStreamHookState,
    probe: &Probe,
) {
    let Err(error) = maybe_raise_for_error_event(hooks, state, &probe.row.event) else {
        return;
    };
    for _attempt in 0..2 {
        handle_failure(hooks, state, callback, &error.exception).await;
    }
}

fn failure_view(error: &StreamFailure, details: &IndexMap<String, Json>) -> Value {
    json!({"status": error.exception.metadata().status_code,
        "pre": fallback_pre(&error.exception),
        "original": error.original_exception.is_some(),
        "billed": details.contains_key("combined_usage_object")})
}

fn fallback_pre(class: &ErrorClass) -> Option<bool> {
    let ErrorClass::MidStreamFallback(fallback) = class else {
        return None;
    };
    Some(fallback.is_pre_first_chunk)
}
