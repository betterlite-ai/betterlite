use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use indexmap::map::Entry;

use crate::converse_content::json_mode::RESPONSE_FORMAT_TOOL_NAME;
use crate::converse_tools::error::ConverseToolsError;

const FAKE_STREAM_MODEL: &str = "meta.llama3-3-70b-instruct-v1:0";

// LiteLLM's _add_tools_to_optional_params: an absent key takes the list, an existing one is extended in place.
pub fn append_tools(
    optional_params: &mut IndexMap<String, Json>,
    tools: Vec<Json>,
) -> Result<(), ConverseToolsError> {
    match optional_params.entry("tools".to_owned()) {
        Entry::Vacant(slot) => {
            slot.insert(Json::List(tools));
        }
        Entry::Occupied(mut slot) => extend_tools(slot.get_mut(), tools)?,
    }
    Ok(())
}

#[must_use]
pub fn create_json_tool_call(json_schema: Option<&Json>, description: Option<&str>) -> Json {
    let parameters = json_schema.map_or_else(empty_json_schema, Clone::clone);
    let mut function = IndexMap::new();
    function.insert(
        "name".to_owned(),
        Json::Text(RESPONSE_FORMAT_TOOL_NAME.to_owned()),
    );
    function.insert("parameters".to_owned(), parameters);
    if let Some(description) = description.filter(|text| !text.is_empty()) {
        function.insert("description".to_owned(), Json::Text(description.to_owned()));
    }
    let mut tool = IndexMap::new();
    tool.insert("type".to_owned(), Json::Text("function".to_owned()));
    tool.insert("function".to_owned(), Json::Object(function));
    Json::Object(tool)
}

pub fn apply_tool_call_transformation(
    tools: &[Json],
    model: &str,
    stream: &Json,
    optional_params: &mut IndexMap<String, Json>,
) -> Result<(), ConverseToolsError> {
    append_tools(optional_params, tools.to_vec())?;
    if model.contains(FAKE_STREAM_MODEL) && matches!(stream, Json::Bool(true)) {
        optional_params.insert("fake_stream".to_owned(), Json::Bool(true));
    }
    Ok(())
}

fn empty_json_schema() -> Json {
    let mut fields = IndexMap::new();
    fields.insert("type".to_owned(), Json::Text("object".to_owned()));
    fields.insert("additionalProperties".to_owned(), Json::Bool(true));
    fields.insert("properties".to_owned(), Json::Object(IndexMap::new()));
    Json::Object(fields)
}

fn extend_tools(existing: &mut Json, tools: Vec<Json>) -> Result<(), ConverseToolsError> {
    let Json::List(existing) = existing else {
        return Err(ConverseToolsError::ToolsNotList);
    };
    existing.extend(tools);
    Ok(())
}
