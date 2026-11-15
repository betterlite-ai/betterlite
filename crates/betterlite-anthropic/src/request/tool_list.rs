use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;

use super::convert;

const TOOLS: &str = "tools";

#[derive(Debug, thiserror::Error)]
pub enum ToolListError {
    #[error("tools must be a list")]
    NotList,
    #[error("tools entry {index} is not an object")]
    NotObject { index: usize },
}

impl ToolListError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::NotList | Self::NotObject { .. } => Some(400),
        }
    }
}

// Null reads as absent, so a request that never set tools reaches the header and advisor scans with none
pub(super) fn tool_entries(
    params: &IndexMap<String, Json>,
) -> Result<Vec<PromptJson>, ToolListError> {
    let entries = match params.get(TOOLS) {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::List(items)) => items,
        Some(_) => return Err(ToolListError::NotList),
    };
    let mut tools = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        if !matches!(entry, Json::Object(_)) {
            return Err(ToolListError::NotObject { index });
        }
        tools.push(convert::prompt_json(entry));
    }
    Ok(tools)
}
