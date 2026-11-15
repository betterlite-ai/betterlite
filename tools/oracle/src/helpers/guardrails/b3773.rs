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

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Flatten(FlattenArgs),
    RestoreShapes(RestoreShapesArgs),
    RestoreProtected(RestoreProtectedArgs),
    RetrieveIds(RetrieveIdsArgs),
    RetrievalResults(MessageIdsArgs),
    ProtectedIndices(MessageIdsArgs),
    CompressFailureDetail(FailureDetailArgs),
    HandleFailure(HandleFailureArgs),
    CcrHashes(CcrHashesArgs),
    Timeout(TimeoutArgs),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlattenArgs {
    rows: Vec<JsonValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestoreShapesArgs {
    originals: Vec<JsonValue>,
    returned: Vec<JsonValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestoreProtectedArgs {
    messages: Vec<JsonValue>,
    compressed: Vec<JsonValue>,
    protected: BTreeSet<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetrieveIdsArgs {
    #[serde(default)]
    messages: Option<JsonValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessageIdsArgs {
    messages: Vec<JsonValue>,
    #[serde(default)]
    extra_ids: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FailureDetailArgs {
    status: u16,
    body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HandleFailureArgs {
    fallback: UnreachableFallback,
    rows: Vec<JsonValue>,
    error: String,
    #[serde(default)]
    detail: IndexMap<String, JsonValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CcrHashesArgs {
    body: JsonValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TimeoutArgs {
    timeout: Option<f64>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Flatten(args) => Ok(json!({ "rows": flatten_for_compression(&args.rows) })),
            Input::RestoreShapes(args) => Ok(json!({
                "rows": restore_content_shapes(&args.originals, &args.returned),
            })),
            Input::RestoreProtected(args) => restore_protected(args),
            Input::RetrieveIds(args) => {
                Ok(json!({ "ids": retrieve_call_ids(args.messages.as_ref()) }))
            }
            Input::RetrievalResults(args) => Ok(json!({
                "indices": retrieval_result_indices(&args.messages, &args.extra_ids),
            })),
            Input::ProtectedIndices(args) => Ok(json!({
                "indices": protected_indices(&args.messages, &args.extra_ids),
            })),
            Input::CompressFailureDetail(args) => Ok(json!({
                "detail": build_compress_failure_detail(args.status, &args.body),
            })),
            Input::HandleFailure(args) => Ok(handle_failure(args)),
            Input::CcrHashes(args) => Ok(json!({ "hashes": read_ccr_hashes(&args.body) })),
            Input::Timeout(args) => Ok(timeout(args)),
        }
    }
}

fn restore_protected(args: &RestoreProtectedArgs) -> HelperResult {
    let rows = restore_protected_messages(&args.messages, &args.compressed, &args.protected)
        .or_value_error()?;
    Ok(json!({ "rows": rows }))
}

fn handle_failure(args: &HandleFailureArgs) -> serde_json::Value {
    match handle_compress_failure(args.fallback, &args.rows, &args.error, &args.detail) {
        Ok(forwarded) => json!({ "rows": forwarded }),
        Err(failure) => json!({ "status": failure.status, "body": failure.body }),
    }
}

fn timeout(args: &TimeoutArgs) -> serde_json::Value {
    let resolved = resolve_timeout(args.timeout);
    json!({
        "total_seconds": resolved.total.as_secs_f64(),
        "connect_seconds": resolved.connect.as_secs_f64(),
    })
}
