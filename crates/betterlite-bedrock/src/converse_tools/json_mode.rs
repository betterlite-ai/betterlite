use betterlite_models::decisions::Json;
use betterlite_openai::base_helpers;
use indexmap::IndexMap;

use crate::converse_content::json_mode::RESPONSE_FORMAT_TOOL_NAME;
use crate::converse_tools::error::ConverseToolsError;

const FAKE_STREAM_MODEL: &str = "meta.llama3-3-70b-instruct-v1:0";

// B2202's base helper owns the append-or-extend rule; the params cross its serde_json map and come back.
pub fn append_tools(
    optional_params: &mut IndexMap<String, Json>,
    tools: Vec<Json>,
) -> Result<(), ConverseToolsError> {
    let wire = serde_json::to_value(&*optional_params).map_err(ConverseToolsError::Wire)?;
    let mut base = serde_json::from_value(wire).map_err(ConverseToolsError::Wire)?;
    let encoded = tools
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ConverseToolsError::Wire)?;
    base_helpers::append_tools(&mut base, encoded)?;
    let rewritten = serde_json::to_value(base).map_err(ConverseToolsError::Wire)?;
    *optional_params = serde_json::from_value(rewritten).map_err(ConverseToolsError::Wire)?;
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
    if let Some(note) = description {
        function.insert("description".to_owned(), Json::Text(note.to_owned()));
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
