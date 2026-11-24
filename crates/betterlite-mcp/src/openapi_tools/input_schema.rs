use betterlite_prompts::arguments::json::{Json, Object};

use crate::openapi_tools::document::{Operation, RequestBody, Schema};

const BODY_DESCRIPTION: &str = "Request body";
const JSON_CONTENT: &str = "application/json";

// LiteLLM's build_input_schema: every named parameter, plus the JSON request body as "body"
#[must_use]
pub fn build_input_schema(operation: &Operation) -> Object {
    let mut properties = Object::new();
    let mut required: Vec<Json> = Vec::new();
    for param in &operation.parameters {
        let Some(name) = param.name.as_deref() else {
            continue;
        };
        let kind = param
            .schema
            .as_ref()
            .and_then(|schema| schema.kind.as_deref())
            .unwrap_or("string");
        properties.insert(
            name.to_owned(),
            parameter_property(kind, param.description.as_deref()),
        );
        if param.required == Some(true) {
            required.push(Json::Text(name.to_owned()));
        }
    }
    if let Some(request_body) = &operation.request_body
        && let Some(media) = request_body.content.get(JSON_CONTENT)
    {
        properties.insert(
            "body".to_owned(),
            body_property(request_body, media.schema.as_ref()),
        );
        if request_body.required == Some(true) {
            required.push(Json::Text("body".to_owned()));
        }
    }
    let mut schema = Object::new();
    schema.insert("type".to_owned(), Json::Text("object".to_owned()));
    schema.insert("properties".to_owned(), Json::Object(properties));
    schema.insert("required".to_owned(), Json::List(required));
    schema
}

fn parameter_property(kind: &str, description: Option<&str>) -> Json {
    let mut property = Object::new();
    property.insert("type".to_owned(), Json::Text(kind.to_owned()));
    property.insert(
        "description".to_owned(),
        Json::Text(description.unwrap_or_default().to_owned()),
    );
    Json::Object(property)
}

fn body_property(request_body: &RequestBody, schema: Option<&Schema>) -> Json {
    let mut property = Object::new();
    property.insert("type".to_owned(), Json::Text("object".to_owned()));
    property.insert(
        "description".to_owned(),
        Json::Text(
            request_body
                .description
                .clone()
                .unwrap_or_else(|| BODY_DESCRIPTION.to_owned()),
        ),
    );
    let properties = schema
        .map(|found| found.properties.clone())
        .unwrap_or_default();
    property.insert("properties".to_owned(), Json::Object(properties));
    Json::Object(property)
}
