use crate::oci_common::OciError;
use crate::oci_common::schema::refs::resolve_oci_schema_refs;
use crate::oci_common::schema::{resolve_oci_schema_anyof, sanitize_oci_schema};
use betterlite_openai::chat::FunctionDefinition;
use betterlite_prompts::arguments::json::{Json, Object};
const PARAMETERS_OBJECT: &str = "Tool `function.parameters` must be an object";
pub(crate) fn resolved_tool_schema(
    function: Option<&FunctionDefinition>,
) -> Result<Json, OciError> {
    // any serde_json value is representable as Json, so the value fallback never runs
    let raw = function
        .and_then(|definition| definition.parameters.as_ref())
        .map_or_else(
            || Json::Object(Object::new()),
            |value| serde_json::from_value(value.clone()).unwrap_or(Json::Null),
        );
    if !matches!(&raw, Json::Object(_)) {
        return Err(OciError {
            status: 400,
            message: String::from(PARAMETERS_OBJECT),
        });
    }
    Ok(sanitize_oci_schema(&resolve_oci_schema_anyof(
        &resolve_oci_schema_refs(&raw)?,
    )))
}
