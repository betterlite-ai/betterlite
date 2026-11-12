use betterlite_vertex::gemini_response_parts::projection::{
    CallMode, LogprobsResult, Part, PartContext, project_parts,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::helpers::{HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    parts: Vec<Part>,
    legacy_function: bool,
    cumulative_tool_call_idx: u32,
    now: i64,
    generated_id: String,
    logprobs: Option<LogprobsResult>,
}

pub(super) fn run(input: &Input) -> HelperResult {
    let uuid = Uuid::parse_str(&input.generated_id).or_value_error()?;
    let context = PartContext {
        cumulative_tool_call_index: input.cumulative_tool_call_idx,
        call_mode: if input.legacy_function {
            CallMode::LegacyFunction
        } else {
            CallMode::Tools
        },
        now: input.now,
    };
    let mut ids = move || uuid;
    let projection =
        project_parts(&input.parts, input.logprobs.as_ref(), context, &mut ids).or_value_error()?;
    serde_json::to_value(&projection).or_value_error()
}
