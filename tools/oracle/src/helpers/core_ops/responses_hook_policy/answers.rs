use std::collections::BTreeSet;

use betterlite_core::api::responses_hook_policy::agentic::get_agentic_loop_settings;
use betterlite_core::api::responses_hook_policy::followup::{
    AgenticFollowupInput, build_agentic_followup_kwargs,
};
use betterlite_core::api::responses_hook_policy::safety::{
    can_finalize_turn, check_agentic_loop_safety, is_refused_tool_use_block, refused_tool_ids,
    sort_json_keys,
};
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::{Input, failure};

pub(super) fn settings_answer(input: &Input) -> Value {
    let Input::Settings { probes } = input else {
        return json!({
            "error": { "class": "InvalidInput", "message": "expected a settings input" },
        });
    };
    let answers: Vec<Value> = probes.iter().map(settings_probe).collect();
    json!({ "probes": answers })
}

fn settings_probe(kwargs: &IndexMap<String, Json>) -> Value {
    match get_agentic_loop_settings(kwargs) {
        Ok(settings) => json!({
            "settings": {
                "depth": settings.depth,
                "max_loops": settings.max_loops,
                "fingerprints": settings.fingerprints,
            },
        }),
        Err(error) => json!({
            "error": { "class": error.python_class(), "message": error.to_string() },
        }),
    }
}

pub(super) fn safety_answer(input: &Input) -> Value {
    let Input::Safety {
        tool_calls,
        fingerprints,
        depth,
        max_loops,
        model,
        blocks,
        stream,
        api_surface,
    } = input
    else {
        return json!({
            "error": { "class": "InvalidInput", "message": "expected a safety input" },
        });
    };
    let refused = refused_tool_ids(tool_calls);
    let ids: Vec<&String> = refused.ids.iter().collect();
    let names: Vec<&String> = refused.names.iter().collect();
    let matched: Vec<bool> = blocks
        .iter()
        .map(|block| is_refused_tool_use_block(block, &refused.ids, &refused.names))
        .collect();
    let canonical = sort_json_keys(tool_calls);
    json!({
        "fingerprint": python_json::dumps(&canonical),
        "safety": safety_probe(tool_calls, fingerprints, *depth, *max_loops, model),
        "refused": { "ids": ids, "names": names },
        "blocks": matched,
        "terminal": can_finalize_turn(*stream, *api_surface),
    })
}

fn safety_probe(
    tool_calls: &Json,
    fingerprints: &[Json],
    depth: i64,
    max_loops: u32,
    model: &str,
) -> Value {
    match check_agentic_loop_safety(tool_calls, fingerprints, depth, max_loops, model) {
        Ok(checked) => json!({ "fingerprint": checked }),
        Err(error) => json!({
            "error": { "class": "AgenticLoopSafetyError", "message": error.to_string() },
        }),
    }
}

pub(super) fn followup_answer(input: &Input) -> crate::helpers::HelperResult {
    let Input::Followup {
        request_kwargs,
        patch_kwargs,
        request_params,
        depth,
        max_loops,
        fingerprints,
        fingerprint,
    } = input
    else {
        return Err(failure(
            "InvalidInput",
            "expected a followup input".to_owned(),
        ));
    };
    let params: BTreeSet<String> = request_params.iter().cloned().collect();
    let agentic = AgenticFollowupInput {
        request_kwargs,
        patch_kwargs,
        request_params: &params,
        depth: *depth,
        max_loops: *max_loops,
        fingerprints,
        fingerprint,
    };
    let kwargs = build_agentic_followup_kwargs(&agentic)
        .map_err(|error| failure("AgenticFollowupKwargsError", error.to_string()))?;
    Ok(json!(kwargs))
}
