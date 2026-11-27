use betterlite_bedrock::batch_call::error::BatchCallError;
use betterlite_bedrock::batch_call::logging::{CallEvent, CallLogger};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::batches::records::BatchObject;
use serde_json::{Value, json};

// the logging capture records what the handler logged instead of running the logger stack
pub(super) fn with_events<Output>(
    enabled: bool,
    events: &mut Vec<Value>,
    invoke: impl FnOnce(Option<&mut CallLogger<'_>>) -> Output,
) -> Output {
    let mut capture = |batch_id: &str, event: CallEvent<'_>| -> Result<(), PrePostError> {
        events.push(event_view(batch_id, &event));
        Ok(())
    };
    if enabled {
        invoke(Some(&mut capture))
    } else {
        invoke(None)
    }
}

// LiteLLM logs the batch id as the input and only the pre event carries an api base
fn event_view(batch_id: &str, event: &CallEvent<'_>) -> Value {
    match event {
        CallEvent::Pre { api_base } => json!({
            "event": "pre",
            "input": batch_id,
            "api_base": api_base,
        }),
        CallEvent::Post { .. } => json!({
            "event": "post",
            "input": batch_id,
            "api_base": Value::Null,
        }),
    }
}

pub(super) fn outcome(result: Result<BatchObject, BatchCallError>) -> (Value, Value) {
    match result {
        Ok(batch) => (projected(&batch), Value::Null),
        Err(error) => (Value::Null, error_view(&error)),
    }
}

fn error_view(error: &BatchCallError) -> Value {
    match error {
        BatchCallError::Client { status, code, .. } => json!({
            "kind": "client",
            "status": status,
            "code": code,
        }),
        BatchCallError::InvalidStatus { .. } => json!({
            "kind": "invalid_status",
            "status": null,
            "code": null,
        }),
        BatchCallError::Decode { .. }
        | BatchCallError::Field { .. }
        | BatchCallError::MissingField { .. }
        | BatchCallError::ResponseShape => json!({
            "kind": "shape",
            "status": null,
            "code": null,
        }),
        BatchCallError::Async(_)
        | BatchCallError::Auth(_)
        | BatchCallError::Credentials(_)
        | BatchCallError::Logging(_)
        | BatchCallError::Region(_)
        | BatchCallError::Signing(_)
        | BatchCallError::Status(_)
        | BatchCallError::Transport(_) => json!({
            "kind": "sdk",
            "status": null,
            "code": null,
        }),
    }
}

fn projected(batch: &BatchObject) -> Value {
    json!({
        "id": &batch.id,
        "object": &batch.object,
        "endpoint": &batch.endpoint,
        "completion_window": &batch.completion_window,
        "status": &batch.status,
        "created_at": batch.created_at,
        "in_progress_at": batch.in_progress_at,
        "completed_at": batch.completed_at,
        "failed_at": batch.failed_at,
        "cancelled_at": batch.cancelled_at,
        "expires_at": batch.expires_at,
        "finalizing_at": batch.finalizing_at,
        "expired_at": batch.expired_at,
        "cancelling_at": batch.cancelling_at,
        "error_file_id": &batch.error_file_id,
        "input_file_id": &batch.input_file_id,
        "output_file_id": &batch.output_file_id,
        "request_counts": &batch.request_counts,
        "errors": &batch.errors,
        "metadata": &batch.metadata,
        "model": &batch.model,
        "usage": &batch.usage,
    })
}
