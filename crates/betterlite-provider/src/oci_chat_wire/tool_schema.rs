use betterlite_openai::chat::FunctionDefinition;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::oci_common::OciError;
use crate::oci_common::schema::refs::resolve_oci_schema_refs;
use crate::oci_common::schema::{resolve_oci_schema_anyof, sanitize_oci_schema};

pub(crate) fn resolved_tool_schema(
    function: Option<&FunctionDefinition>,
) -> Result<Json, OciError> {
    let raw = match function.and_then(|definition| definition.parameters.as_ref()) {
        Some(value) => serde_json::from_value(value.clone()).map_err(|source| OciError {
            status: 400,
            message: format!("Tool `function.parameters` is not a JSON schema: {source}"),
        })?,
        None => Json::Object(Object::new()),
    };
    if !matches!(&raw, Json::Object(_)) {
        return Err(OciError {
            status: 400,
            message: String::from("Tool `function.parameters` must be an object"),
        });
    }
    Ok(sanitize_oci_schema(&resolve_oci_schema_anyof(
        &resolve_oci_schema_refs(&raw)?,
    )))
}
