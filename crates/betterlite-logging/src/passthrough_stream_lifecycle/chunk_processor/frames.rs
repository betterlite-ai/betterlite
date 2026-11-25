use std::mem::take;

use betterlite_models::decisions::Json;

use crate::passthrough_stream_lifecycle::cost_injection::cost_injection_active;
use crate::passthrough_stream_lifecycle::lifecycle::StreamLifecycle;
// ast-grep-ignore: no-long-names
use crate::passthrough_stream_logging::model::extract_model_for_cost_injection;
use crate::passthrough_stream_logging::object::StreamLoggingObject;
use crate::passthrough_stream_logging::sse_frames::split_complete_sse_frames;
use crate::passthrough_stream_logging::types::EndpointType;

use super::GuardParts;

const RUN_ID_KEY: &[u8] = b"run_id";

pub(super) fn resolve_injection_model(
    lifecycle: &StreamLifecycle,
    logging_obj: &StreamLoggingObject,
    endpoint_type: EndpointType,
    url_route: &str,
    request_body: &Json,
) -> Option<String> {
    let model_from_details = logging_obj
        .read(&mut |state| match state.model_call_details.get("model") {
            Some(Json::Text(model)) => Some(model.clone()),
            Some(_) | None => None,
        })
        .flatten();
    // ast-grep-ignore: no-long-names
    let model_name = extract_model_for_cost_injection(
        Some(request_body),
        model_from_details.as_deref(),
        url_route,
        endpoint_type,
    );
    let model = model_name.as_deref()?;
    if !cost_injection_active(
        lifecycle.cost_injection_enabled,
        Some(model),
        endpoint_type,
        url_route,
    ) {
        return None;
    }
    Some(model.to_owned())
}

pub(super) fn inject_frames(parts: &mut GuardParts, chunk: &[u8]) -> Option<Vec<u8>> {
    parts.pending.extend_from_slice(chunk);
    let (complete, tail) = split_complete_sse_frames(&parts.pending);
    let frames = complete.to_vec();
    let remaining = tail.to_vec();
    parts.pending = remaining;
    if frames.is_empty() {
        return None;
    }
    let model = parts.injection_model.as_deref()?;
    Some(
        parts
            .lifecycle
            .cost_injection
            .inject(&frames, model, &parts.logging_obj),
    )
}

pub(super) fn scan_tinyfish(parts: &mut GuardParts, chunk: &[u8]) {
    if !parts.scan_active {
        return;
    }
    let mut reassembled = take(&mut parts.scan_pending);
    reassembled.extend_from_slice(chunk);
    let (complete, tail) = split_complete_sse_frames(&reassembled);
    if contains_run_id(complete)
        && let Some(run_id) = parts
            .lifecycle
            .router
            .tinyfish
            .run_id_from_sse_frames(complete)
    {
        parts.lifecycle.router.tinyfish.start_sse_run_billing(
            &run_id,
            &parts.logging_obj,
            parts.start_time,
        );
        parts.scan_active = false;
    }
    parts.scan_pending = tail.to_vec();
}

fn contains_run_id(frames: &[u8]) -> bool {
    frames
        .windows(RUN_ID_KEY.len())
        .any(|window| window == RUN_ID_KEY)
}
