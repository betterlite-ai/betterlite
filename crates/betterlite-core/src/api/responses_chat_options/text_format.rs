use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

const RESPONSE_SCHEMA: &str = "response_schema";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonSchemaFormat {
    pub name: Json,
    pub schema: Json,
    pub strict: Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    JsonSchema { json_schema: Box<JsonSchemaFormat> },
    JsonObject,
}

// absent, null, an empty object and unknown format types all leave the request without a response format
pub(crate) fn response_format(text: Option<&Json>) -> Option<ResponseFormat> {
    let Json::Object(text_fields) = text? else {
        return None;
    };
    let Some(Json::Object(format)) = text_fields.get("format") else {
        return None;
    };
    let Some(Json::Text(kind)) = format.get("type") else {
        return None;
    };
    match kind.as_str() {
        "json_schema" => Some(ResponseFormat::JsonSchema {
            json_schema: Box::new(JsonSchemaFormat {
                name: format
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| Json::Text(RESPONSE_SCHEMA.to_owned())),
                schema: format
                    .get("schema")
                    .cloned()
                    .unwrap_or_else(|| Json::Object(IndexMap::new())),
                strict: format.get("strict").cloned().unwrap_or(Json::Bool(false)),
            }),
        }),
        "json_object" => Some(ResponseFormat::JsonObject),
        _ => None,
    }
}
