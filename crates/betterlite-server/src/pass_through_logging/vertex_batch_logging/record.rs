use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::{VertexBatchInput, VertexBatchKwargs, VertexBatchLogged, VertexBatchState};
use crate::pass_through_logging::anthropic_batch::{
    BatchLoggingChoice, BatchLoggingMessage, BatchLoggingResponse,
};

const FAILED_MODEL: &str = "vertex_ai_batch";
const RECORD_OBJECT: &str = "batch_prediction_job";

pub(super) struct Pending {
    pub(super) batch_id: String,
    pub(super) model: String,
    pub(super) unified_object_id: String,
}

pub(super) enum Outcome {
    Pending(Pending),
    Rejected(u16),
    Errored(String),
}

struct Parts {
    model: String,
    content: String,
    state: VertexBatchState,
    fields: IndexMap<String, Json>,
    ids: Option<(String, String)>,
}

impl VertexBatchState {
    const fn word(self) -> &'static str {
        match self {
            Self::JobStatePending => "JOB_STATE_PENDING",
            Self::JobStateFailed => "JOB_STATE_FAILED",
        }
    }
}

fn parts(outcome: Outcome) -> Parts {
    match outcome {
        Outcome::Pending(pending) => {
            let state = VertexBatchState::JobStatePending;
            let content = format!(
                "Batch prediction job {} created and is pending. Status will be updated when the batch completes.",
                pending.batch_id
            );
            let mut fields = IndexMap::new();
            fields.insert(
                "batch_job_id".to_owned(),
                Json::Text(pending.batch_id.clone()),
            );
            fields.insert(
                "batch_job_state".to_owned(),
                Json::Text(state.word().to_owned()),
            );
            let unified = Json::Text(pending.unified_object_id.clone());
            fields.insert("unified_object_id".to_owned(), unified);
            let ids = Some((pending.batch_id, pending.unified_object_id));
            Parts {
                model: pending.model,
                content,
                state,
                fields,
                ids,
            }
        }
        Outcome::Rejected(status_code) => failed(
            format!("Batch prediction job creation failed. Status: {status_code}"),
            "status_code",
            Json::Number(Number::from(status_code)),
        ),
        Outcome::Errored(error) => failed(
            format!("Error creating batch prediction job: {error}"),
            "error",
            Json::Text(error),
        ),
    }
}

fn failed(content: String, name: &str, value: Json) -> Parts {
    let state = VertexBatchState::JobStateFailed;
    let mut fields = IndexMap::new();
    fields.insert(
        "batch_job_state".to_owned(),
        Json::Text(state.word().to_owned()),
    );
    fields.insert(name.to_owned(), value);
    Parts {
        model: FAILED_MODEL.to_owned(),
        content,
        state,
        fields,
        ids: None,
    }
}

pub(super) fn logged(
    input: &VertexBatchInput<'_>,
    outcome: Outcome,
    response_id: String,
) -> VertexBatchLogged {
    let record = parts(outcome);
    let message = BatchLoggingMessage {
        role: "assistant".to_owned(),
        content: record.content,
        tool_calls: None,
        function_call: None,
        provider_specific_fields: record.fields,
    };
    let (batch_id, unified_object_id) = record.ids.unzip();
    VertexBatchLogged {
        result: BatchLoggingResponse {
            id: response_id,
            model: record.model.clone(),
            object: RECORD_OBJECT.to_owned(),
            created: input.start_time.timestamp(),
            choices: vec![BatchLoggingChoice {
                finish_reason: "stop".to_owned(),
                index: 0,
                message,
            }],
        },
        kwargs: VertexBatchKwargs {
            response_cost: Decimal::ZERO,
            model: record.model,
            batch_id,
            unified_object_id,
            batch_job_state: record.state,
            request_metadata: input.request_metadata.clone(),
        },
    }
}
