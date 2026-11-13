use serde::{Deserialize, Serialize};

use crate::error_responses::{error_response_schema, error_responses};
use crate::llm_request_bodies::{LlmRequestSchemas, add_llm_request_bodies};
use crate::operation_ids::{ensure_unique_operation_ids, stable_operation_id};
use crate::schema_transforms::tree::{JsonObject, JsonTree};
use crate::websocket_stubs::{WebsocketRoute, inject_websocket_stubs};

mod tests;

const CUSTOMIZE_SWAGGER_LINK: &str = "[**Customize Swagger Docs**](https://docs.litellm.ai/docs/proxy/enterprise#swagger-docs---custom-routes--branding)";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteDoc {
    pub path: String,
    pub method: String,
    pub name: String,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub parameters: Vec<ParamDoc>,
    pub request_body: Option<JsonObject>,
    pub error_responses: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamDoc {
    pub name: String,
    pub location: String,
    pub required: bool,
    pub schema: JsonObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenApiInfo {
    pub title: String,
    pub version: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentInputs {
    pub info: OpenApiInfo,
    pub routes: Vec<RouteDoc>,
    pub websockets: Vec<WebsocketRoute>,
    pub llm_schemas: LlmRequestSchemas,
    pub server_root_path: String,
}

#[must_use]
pub fn default_info(server_root_path: &str, version: &str) -> OpenApiInfo {
    let ui_message = format!(
        "\u{1F449} [```LiteLLM Admin Panel on /ui```]({server_root_path}/ui). Create, Edit Keys with SSO. Having issues? Try [```Fallback Login```]({server_root_path}/fallback/login)\n\n\u{1F4B8} [```LiteLLM Model Cost Map```](https://models.litellm.ai/).\n\n\u{1F50E} [```LiteLLM Model Hub```]({server_root_path}/ui/model_hub_table). See available models on the proxy. [**Docs**](https://docs.litellm.ai/docs/proxy/ai_hub)"
    );
    OpenApiInfo {
        title: "LiteLLM API".to_owned(),
        version: version.to_owned(),
        description: format!(
            "Proxy Server to call 100+ LLMs in the OpenAI format. {CUSTOMIZE_SWAGGER_LINK}\n\n{ui_message}"
        ),
    }
}

#[must_use]
pub fn build_document(inputs: &DocumentInputs) -> JsonObject {
    let mut document = JsonObject::from([
        ("openapi".to_owned(), JsonTree::Text("3.1.0".to_owned())),
        (
            "info".to_owned(),
            JsonTree::Object(info_object(&inputs.info)),
        ),
        (
            "paths".to_owned(),
            JsonTree::Object(build_paths(&inputs.routes)),
        ),
        (
            "components".to_owned(),
            JsonTree::Object(JsonObject::from([(
                "schemas".to_owned(),
                JsonTree::Object(build_schemas(&inputs.routes)),
            )])),
        ),
    ]);
    inject_websocket_stubs(&mut document, &inputs.websockets);
    add_llm_request_bodies(&mut document, &inputs.llm_schemas);
    ensure_unique_operation_ids(&mut document, None);
    if !inputs.server_root_path.is_empty() {
        let url = format!("/{}", inputs.server_root_path.trim_matches('/'));
        document.insert(
            "servers".to_owned(),
            JsonTree::Array(vec![JsonTree::Object(JsonObject::from([(
                "url".to_owned(),
                JsonTree::Text(url),
            )]))]),
        );
    }
    document
}

fn info_object(info: &OpenApiInfo) -> JsonObject {
    JsonObject::from([
        ("title".to_owned(), JsonTree::Text(info.title.clone())),
        ("version".to_owned(), JsonTree::Text(info.version.clone())),
        (
            "description".to_owned(),
            JsonTree::Text(info.description.clone()),
        ),
    ])
}

fn build_paths(routes: &[RouteDoc]) -> JsonObject {
    let mut paths = JsonObject::new();
    for route in routes {
        add_route(&mut paths, route);
    }
    paths
}

fn add_route(paths: &mut JsonObject, route: &RouteDoc) {
    let method = route.method.to_lowercase();
    let operation = build_operation(route, &method);
    let path_entry = paths
        .entry(route.path.clone())
        .or_insert_with(|| JsonTree::Object(JsonObject::new()));
    let JsonTree::Object(path_operations) = path_entry else {
        return;
    };
    path_operations.insert(method, JsonTree::Object(operation));
}

fn build_schemas(routes: &[RouteDoc]) -> JsonObject {
    if routes.iter().any(|route| route.error_responses) {
        JsonObject::from([(
            "ErrorResponse".to_owned(),
            JsonTree::Object(error_response_schema()),
        )])
    } else {
        JsonObject::new()
    }
}

fn build_operation(route: &RouteDoc, method: &str) -> JsonObject {
    let mut operation = JsonObject::new();
    if !route.tags.is_empty() {
        operation.insert("tags".to_owned(), tag_array(&route.tags));
    }
    let summary = route
        .summary
        .as_ref()
        .map_or_else(|| title_case(&route.name.replace('_', " ")), String::clone);
    operation.insert("summary".to_owned(), JsonTree::Text(summary));
    operation.insert(
        "operationId".to_owned(),
        JsonTree::Text(stable_operation_id(&route.name, &route.path, &[method])),
    );
    if !route.parameters.is_empty() {
        operation.insert("parameters".to_owned(), parameter_array(&route.parameters));
    }
    if let Some(request_body) = &route.request_body {
        operation.insert(
            "requestBody".to_owned(),
            JsonTree::Object(request_body.clone()),
        );
    }
    operation.insert(
        "responses".to_owned(),
        JsonTree::Object(build_responses(route.error_responses)),
    );
    operation
}

fn tag_array(tags: &[String]) -> JsonTree {
    let mut items = Vec::with_capacity(tags.len());
    for tag in tags {
        items.push(JsonTree::Text(tag.clone()));
    }
    JsonTree::Array(items)
}

fn parameter_array(parameters: &[ParamDoc]) -> JsonTree {
    let mut items = Vec::with_capacity(parameters.len());
    for parameter in parameters {
        items.push(parameter_json(parameter));
    }
    JsonTree::Array(items)
}

fn parameter_json(parameter: &ParamDoc) -> JsonTree {
    JsonTree::Object(JsonObject::from([
        ("name".to_owned(), JsonTree::Text(parameter.name.clone())),
        ("in".to_owned(), JsonTree::Text(parameter.location.clone())),
        ("required".to_owned(), JsonTree::Flag(parameter.required)),
        (
            "schema".to_owned(),
            JsonTree::Object(parameter.schema.clone()),
        ),
    ]))
}

fn build_responses(with_errors: bool) -> JsonObject {
    let success = response_json("Successful Response", JsonTree::Object(JsonObject::new()));
    let mut responses = JsonObject::from([("200".to_owned(), success)]);
    if with_errors {
        for error in error_responses() {
            responses.insert(
                error.status.to_string(),
                response_json(&error.description, error_schema_ref()),
            );
        }
    }
    responses
}

fn response_json(description: &str, schema: JsonTree) -> JsonTree {
    JsonTree::Object(JsonObject::from([
        (
            "description".to_owned(),
            JsonTree::Text(description.to_owned()),
        ),
        (
            "content".to_owned(),
            JsonTree::Object(JsonObject::from([(
                "application/json".to_owned(),
                JsonTree::Object(JsonObject::from([("schema".to_owned(), schema)])),
            )])),
        ),
    ]))
}

fn error_schema_ref() -> JsonTree {
    JsonTree::Object(JsonObject::from([(
        "$ref".to_owned(),
        JsonTree::Text("#/components/schemas/ErrorResponse".to_owned()),
    )]))
}

// Python's title() uppercases a cased character only after an uncased one
fn title_case(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut previous_is_cased = false;
    for character in text.chars() {
        if character.is_alphabetic() {
            if previous_is_cased {
                output.extend(character.to_lowercase());
            } else {
                output.extend(character.to_uppercase());
            }
            previous_is_cased = true;
        } else {
            output.push(character);
            previous_is_cased = false;
        }
    }
    output
}
