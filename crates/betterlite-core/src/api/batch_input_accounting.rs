pub mod calls;
pub mod error;
pub mod input;
pub mod ports;
pub mod result;
pub mod service;

mod access;
mod decode;
mod logging;
mod rows;
mod totals;

mod tests;

use crate::api::api_boundaries::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, EventBoundary, ItemsBoundary, JsonBoundary,
};
use crate::api::request_destination::RequestData;

// One stripped JSONL input line: a parsed object entry, or the raw bytes every fallback estimate counts
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedBatchInputLine<'raw> {
    Entry { entry: RequestData, raw: &'raw [u8] },
    Fallback { raw: &'raw [u8] },
}

// LiteLLM's _iter_batch_input_lines, with the json.loads of each non-empty line done up front
pub fn decode_batch_input_accounting<'raw>(
    bytes: &'raw [u8],
    limits: &ApiLimits,
) -> Result<Vec<DecodedBatchInputLine<'raw>>, ApiLimitError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    let mut lines = Vec::new();
    for raw in bytes.split(|byte| *byte == b'\n') {
        let line = decode::stripped_line(raw);
        if line.is_empty() {
            continue;
        }
        limits.check(ApiBoundariesInput::Event(EventBoundary {
            known_bytes: line.len(),
        }))?;
        limits.check(ApiBoundariesInput::Items(ItemsBoundary {
            known_count: lines.len().saturating_add(1),
        }))?;
        lines.push(decode::decode_line(line));
    }
    Ok(lines)
}
