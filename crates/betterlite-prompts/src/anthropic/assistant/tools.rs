use std::mem::take;

use crate::anthropic_invokes::invoke::to_anthropic_tool_invokes;
use crate::arguments::json::{Json, Object};

use super::fields::server_results;
use super::{
    AssistantError, AssistantRun, ID, ParsedAssistant, SERVER_TOOL_USE, TOOL_RESULTS, TOOL_USE,
    TYPE, WEB_SEARCH_RESULTS,
};

pub(super) fn push_unique(run: &mut AssistantRun, blocks: &[Object]) {
    for block in blocks {
        let Some(id) = block
            .get(ID)
            .and_then(Json::as_str)
            .filter(|id| !id.is_empty())
        else {
            run.blocks.push(Json::Object(block.clone()));
            continue;
        };
        if run.unique_tool_ids.insert(id.to_owned()) {
            run.blocks.push(Json::Object(block.clone()));
        }
    }
}

pub(super) fn convert_tool_calls(
    parsed: &ParsedAssistant<'_>,
) -> Result<Vec<Object>, AssistantError> {
    let web_search_results = server_results(parsed.provider, WEB_SEARCH_RESULTS, parsed.index)?;
    let tool_results = server_results(parsed.provider, TOOL_RESULTS, parsed.index)?;
    to_anthropic_tool_invokes(
        parsed.calls.unwrap_or_default(),
        web_search_results,
        tool_results,
    )
    .map_err(AssistantError::ToolInvoke)
}

pub(super) fn group_tool_invokes(invokes: Vec<Object>) -> (Vec<Vec<Object>>, Vec<Object>) {
    let mut groups: Vec<Vec<Object>> = Vec::new();
    let mut current: Vec<Object> = Vec::new();
    let mut regular: Vec<Object> = Vec::new();
    for block in invokes {
        match block.get(TYPE).and_then(Json::as_str).unwrap_or_default() {
            SERVER_TOOL_USE => {
                if !current.is_empty() {
                    groups.push(take(&mut current));
                }
                current.push(block);
            }
            TOOL_USE => regular.push(block),
            _ => current.push(block),
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    (groups, regular)
}
