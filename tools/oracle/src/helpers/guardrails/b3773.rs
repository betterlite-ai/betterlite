use std::collections::BTreeSet;

use betterlite_guardrails::headroom_failure::{
    build_compress_failure_detail, handle_compress_failure, read_ccr_hashes,
};
use betterlite_guardrails::headroom_messages::{
    flatten_for_compression, protected_indices, restore_content_shapes, restore_protected_messages,
    retrieval_result_indices, retrieve_call_ids,
};
use betterlite_guardrails::headroom_settings::{UnreachableFallback, resolve_timeout};
use betterlite_guardrails::primitives::json_value::JsonValue;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperResult, OrHelperError};

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Flatten {
        rows: Vec<JsonValue>,
    },
    RestoreShapes {
        originals: Vec<JsonValue>,
        returned: Vec<JsonValue>,
    },
    RestoreProtected {
        messages: Vec<JsonValue>,
        compressed: Vec<JsonValue>,
        protected: Vec<usize>,
    },
    RetrieveIds {
        #[serde(default)]
        messages: Option<JsonValue>,
    },
    RetrievalResults {
        messages: Vec<JsonValue>,
        #[serde(default)]
        extra_ids: Vec<String>,
    },
    ProtectedIndices {
        messages: Vec<JsonValue>,
        #[serde(default)]
        extra_ids: Vec<String>,
    },
    CompressFailureDetail {
        status: u16,
        body: String,
    },
    HandleFailure {
        fallback: UnreachableFallback,
        rows: Vec<JsonValue>,
        error: String,
        #[serde(default)]
        detail: IndexMap<String, JsonValue>,
    },
    CcrHashes {
        body: JsonValue,
    },
    Timeout {
        timeout: Option<f64>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Flatten { rows } => Ok(json!({ "rows": flatten_for_compression(rows) })),
            Input::RestoreShapes {
                originals,
                returned,
            } => Ok(json!({
                "rows": restore_content_shapes(originals, returned),
            })),
            Input::RestoreProtected {
                messages,
                compressed,
                protected,
            } => {
                let protected: BTreeSet<usize> = protected.iter().copied().collect();
                let rows = restore_protected_messages(messages, compressed, &protected)
                    .or_value_error()?;
                Ok(json!({ "rows": rows }))
            }
            Input::RetrieveIds { messages } => {
                let ids = retrieve_call_ids(messages.as_ref());
                Ok(json!({ "ids": ids.iter().collect::<Vec<_>>() }))
            }
            Input::RetrievalResults {
                messages,
                extra_ids,
            } => {
                let extra_ids: BTreeSet<String> = extra_ids.iter().cloned().collect();
                let indices: Vec<usize> = retrieval_result_indices(messages, &extra_ids)
                    .into_iter()
                    .collect();
                Ok(json!({ "indices": indices }))
            }
            Input::ProtectedIndices {
                messages,
                extra_ids,
            } => {
                let extra_ids: BTreeSet<String> = extra_ids.iter().cloned().collect();
                let indices: Vec<usize> = protected_indices(messages, &extra_ids)
                    .into_iter()
                    .collect();
                Ok(json!({ "indices": indices }))
            }
            Input::CompressFailureDetail { status, body } => Ok(json!({
                "detail": build_compress_failure_detail(*status, body),
            })),
            Input::HandleFailure {
                fallback,
                rows,
                error,
                detail,
            } => match handle_compress_failure(*fallback, rows, error, detail) {
                Ok(forwarded) => Ok(json!({ "rows": forwarded })),
                Err(failure) => Ok(json!({ "status": failure.status, "body": failure.body })),
            },
            Input::CcrHashes { body } => {
                let hashes: Vec<String> = read_ccr_hashes(body).into_iter().collect();
                Ok(json!({ "hashes": hashes }))
            }
            Input::Timeout { timeout } => {
                let timeout = resolve_timeout(*timeout);
                Ok(json!({
                    "total_seconds": timeout.total.as_secs_f64(),
                    "connect_seconds": timeout.connect.as_secs_f64(),
                }))
            }
        }
    }
}
