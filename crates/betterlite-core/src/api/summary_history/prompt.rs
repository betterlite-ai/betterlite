use betterlite_prompts::arguments::json::Json;
use regex::Regex;

use crate::api::messages_context_clear::constants::{
    COMPACT_DEFAULT_INSTRUCTIONS, COMPACT_NO_TOOL_CALLS_SUFFIX,
};
use crate::api::summary_history::error::SummaryHistoryError;

pub const SUMMARY_TAG_PATTERN: &str = "(?is)<summary>(.*?)</summary>";

#[must_use]
pub fn build_summary_prompt(instructions: Option<&Json>, tools: Option<&[Json]>) -> String {
    if let Some(Json::Text(custom)) = instructions
        && !custom.trim().is_empty()
    {
        return custom.clone();
    }
    if tools.is_some_and(|declared| !declared.is_empty()) {
        format!("{COMPACT_DEFAULT_INSTRUCTIONS}{COMPACT_NO_TOOL_CALLS_SUFFIX}")
    } else {
        COMPACT_DEFAULT_INSTRUCTIONS.to_owned()
    }
}

// only the first tagged span counts, so a blank first summary fails even when a later one has text
pub fn extract_summary_text(raw: Option<&str>) -> Result<Option<String>, SummaryHistoryError> {
    let Some(text) = raw else {
        return Ok(None);
    };
    let pattern = Regex::new(SUMMARY_TAG_PATTERN).map_err(SummaryHistoryError::Pattern)?;
    Ok(pattern
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|summary| summary.as_str().trim())
        .filter(|summary| !summary.is_empty())
        .map(str::to_owned))
}
