use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub const NORMALIZED_REGEX_TYPE: &str = "tool_search_tool_regex";
pub const TOOL_SEARCH_BM25: &str = "tool_search_tool_bm25_20251119";
pub const TOOL_SEARCH_REGEX: &str = "tool_search_tool_regex_20251119";

const NAME: &str = "name";
const TOOLS: &str = "tools";
const TYPE: &str = "type";

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum ToolSearchToolsError {
    #[error("tools[{index}] is not an object")]
    NotObject { index: usize },
}

impl ToolSearchToolsError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::NotObject { .. } => 400,
        }
    }
}

pub fn normalize_tool_search_tools(
    optional_params: &mut IndexMap<String, Json>,
) -> Result<(), ToolSearchToolsError> {
    let Some(Json::List(tools)) = optional_params.get(TOOLS) else {
        return Ok(());
    };
    if tools.is_empty() {
        return Ok(());
    }
    let mut normalized: Vec<Json> = Vec::with_capacity(tools.len());
    for (index, tool) in tools.iter().enumerate() {
        let Json::Object(fields) = tool else {
            return Err(ToolSearchToolsError::NotObject { index });
        };
        match fields.get(TYPE) {
            Some(Json::Text(kind)) if kind.as_str() == TOOL_SEARCH_BM25 => {}
            Some(Json::Text(kind)) if kind.as_str() == TOOL_SEARCH_REGEX => {
                normalized.push(normalized_regex(fields));
            }
            Some(_) | None => normalized.push(tool.clone()),
        }
    }
    optional_params.insert(TOOLS.to_owned(), Json::List(normalized));
    Ok(())
}

// The copy keeps every other field, and an explicit name, null included, stays as sent.
fn normalized_regex(fields: &IndexMap<String, Json>) -> Json {
    let mut rewritten = fields.clone();
    rewritten.insert(
        TYPE.to_owned(),
        Json::Text(NORMALIZED_REGEX_TYPE.to_owned()),
    );
    if !rewritten.contains_key(NAME) {
        rewritten.insert(
            NAME.to_owned(),
            Json::Text(NORMALIZED_REGEX_TYPE.to_owned()),
        );
    }
    Json::Object(rewritten)
}
