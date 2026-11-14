use betterlite_models::model_capability_predicates::CapabilityPredicates;
use betterlite_openai::chat::{ChatMessage, Role, Tool};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Number;

use crate::oci_chat::config::{
    OciParam, get_vendor_from_model, param_map, uses_max_completion_tokens,
};
use crate::oci_chat_wire::cohere::request::{
    adapt_cohere_messages, adapt_cohere_tools, extract_text_content,
};
use crate::oci_chat_wire::generic::request::{adapt_generic_messages, adapt_generic_tools};
use crate::oci_chat_wire::types::cohere::CohereChatRequest;
use crate::oci_chat_wire::types::oci::{OciChatRequestPayload, OciVendor};
use crate::oci_common::credentials::OciCredentials;
use crate::oci_common::{OciError, OciServingMode};

pub const DEFAULT_MAX_TOKENS: i64 = 4096;

const MISSING_COMPARTMENT: &str = "oci_compartment_id is required for OCI chat requests. Pass it as optional_params or set the OCI_COMPARTMENT_ID env var.";

const BAD_SERVING_MODE: &str = "kwarg `oci_serving_mode` must be either 'ON_DEMAND' or 'DEDICATED'";

const NO_USER_MESSAGE: &str =
    "No user message found: Cohere models require at least one user message";

const MISSING_FUNCTION_NAME: &str =
    "Invalid tool_choice for OCI: 'FUNCTION' type requires a non-empty function name";

const OK_TOOL_CHOICE_TYPES: [&str; 3] = ["AUTO", "NONE", "REQUIRED"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciCompletionPayload {
    pub compartment_id: String,
    pub serving_mode: OciServingMode,
    pub chat_request: OciChatRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum OciChatRequest {
    Cohere(CohereChatRequest),
    Generic(OciChatRequestPayload),
}

pub async fn transform_request<Predicates>(
    model: &str,
    messages: &[ChatMessage],
    optional_params: &Object,
    credentials: &OciCredentials,
    predicates: &mut Predicates,
) -> Result<OciCompletionPayload, OciError>
where
    Predicates: CapabilityPredicates + Send,
{
    let Some(compartment_id) = credentials
        .oci_compartment_id
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return Err(OciError {
            status: 400,
            message: MISSING_COMPARTMENT.to_owned(),
        });
    };
    let vendor = get_vendor_from_model(model);
    let serving_mode = serving_mode(model, optional_params)?;
    let selected = get_optional_params(vendor, optional_params, model, predicates).await?;
    let chat_request = chat_request(vendor, messages, &selected)?;
    Ok(OciCompletionPayload {
        compartment_id: compartment_id.to_owned(),
        serving_mode,
        chat_request,
    })
}

fn serving_mode(model: &str, optional_params: &Object) -> Result<OciServingMode, OciError> {
    let mode = match optional_params.get("oci_serving_mode") {
        None | Some(Json::Null) => "ON_DEMAND",
        Some(Json::Text(text)) => text.as_str(),
        Some(_) => {
            return Err(OciError {
                status: 400,
                message: BAD_SERVING_MODE.to_owned(),
            });
        }
    };
    match mode {
        "ON_DEMAND" => Ok(OciServingMode::OnDemand {
            model_id: model.to_owned(),
        }),
        "DEDICATED" => Ok(OciServingMode::Dedicated {
            endpoint_id: endpoint_id(model, optional_params)?,
        }),
        _ => Err(OciError {
            status: 400,
            message: BAD_SERVING_MODE.to_owned(),
        }),
    }
}

fn endpoint_id(model: &str, optional_params: &Object) -> Result<String, OciError> {
    match optional_params.get("oci_endpoint_id") {
        None | Some(Json::Null) => Ok(model.to_owned()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(_) => Err(OciError {
            status: 400,
            message: String::from("kwarg `oci_endpoint_id` must be a string"),
        }),
    }
}

async fn get_optional_params<Predicates>(
    vendor: OciVendor,
    optional_params: &Object,
    model: &str,
    predicates: &mut Predicates,
) -> Result<Object, OciError>
where
    Predicates: CapabilityPredicates + Send,
{
    let token_key = choose_token_key(vendor, model, predicates).await;
    let mut selected = select_params(vendor, optional_params, token_key);
    normalize_reasoning_effort(&mut selected);
    normalize_tools(&mut selected, vendor)?;
    normalize_tool_choice(&mut selected)?;
    normalize_response_format(&mut selected, vendor)?;
    Ok(selected)
}

async fn choose_token_key<Predicates>(
    vendor: OciVendor,
    model: &str,
    predicates: &mut Predicates,
) -> &'static str
where
    Predicates: CapabilityPredicates + Send,
{
    if vendor == OciVendor::Cohere
        || model.is_empty()
        || !uses_max_completion_tokens(model, predicates).await
    {
        "maxTokens"
    } else {
        "maxCompletionTokens"
    }
}

fn select_params(vendor: OciVendor, optional_params: &Object, token_key: &str) -> Object {
    let mut selected = Object::new();
    for (openai_key, alias) in param_map(vendor) {
        let OciParam::Oci(oci_alias) = alias else {
            continue;
        };
        let target = if oci_alias == "maxTokens" {
            token_key
        } else {
            oci_alias
        };
        if selected.contains_key(target) {
            continue;
        }
        if let Some(value) = optional_params
            .get(openai_key)
            .or_else(|| optional_params.get(oci_alias))
        {
            selected.insert(target.to_owned(), value.clone());
        }
    }
    if !selected.contains_key(token_key) {
        selected.insert(
            token_key.to_owned(),
            Json::Number(Number::from(DEFAULT_MAX_TOKENS)),
        );
    }
    selected
}

fn normalize_reasoning_effort(selected: &mut Object) {
    let Some(Json::Text(effort)) = selected.get("reasoningEffort") else {
        return;
    };
    let upper = effort.to_uppercase();
    let normalized = if upper == "DISABLE" {
        String::from("NONE")
    } else {
        upper
    };
    selected.insert(String::from("reasoningEffort"), Json::Text(normalized));
}

fn normalize_tools(selected: &mut Object, vendor: OciVendor) -> Result<(), OciError> {
    let Some(tools) = selected.get("tools").cloned() else {
        return Ok(());
    };
    let bytes = serde_json::to_vec(&tools).map_err(|source| encode_error(&source))?;
    let parsed: Vec<Tool> = serde_json::from_slice(&bytes).map_err(|source| OciError {
        status: 400,
        message: format!("`tools` is not a list of OpenAI tool definitions: {source}"),
    })?;
    let adapted = match vendor {
        OciVendor::Cohere => json_of(&adapt_cohere_tools(&parsed)?)?,
        OciVendor::Generic => json_of(&adapt_generic_tools(&parsed)?)?,
    };
    selected.insert(String::from("tools"), adapted);
    Ok(())
}

fn chat_request(
    vendor: OciVendor,
    messages: &[ChatMessage],
    selected: &Object,
) -> Result<OciChatRequest, OciError> {
    match vendor {
        OciVendor::Cohere => Ok(OciChatRequest::Cohere(cohere_request(messages, selected)?)),
        OciVendor::Generic => Ok(OciChatRequest::Generic(generic_request(
            messages, selected,
        )?)),
    }
}

fn cohere_request(
    messages: &[ChatMessage],
    selected: &Object,
) -> Result<CohereChatRequest, OciError> {
    let Some(last_user) = messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)
    else {
        return Err(OciError {
            status: 400,
            message: NO_USER_MESSAGE.to_owned(),
        });
    };
    let preamble = cohere_preamble(messages);
    let history = cohere_history(messages);
    let mut fields = selected.clone();
    fields.insert(
        String::from("apiFormat"),
        Json::Text(OciVendor::Cohere.api_format().to_owned()),
    );
    fields.insert(
        String::from("message"),
        Json::Text(extract_text_content(last_user.content.as_ref())),
    );
    fields.insert(
        String::from("chatHistory"),
        json_of(&adapt_cohere_messages(&history))?,
    );
    if !preamble.is_empty() {
        fields.insert(String::from("preambleOverride"), Json::Text(preamble));
    }
    typed_request(fields, "CohereChatRequest")
}

fn cohere_preamble(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .filter(|message| message.role == Role::System)
        .map(|message| extract_text_content(message.content.as_ref()))
        .collect::<Vec<String>>()
        .join("\n")
}

fn cohere_history(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    messages
        .iter()
        .filter(|message| message.role != Role::System)
        .cloned()
        .collect()
}

fn generic_request(
    messages: &[ChatMessage],
    selected: &Object,
) -> Result<OciChatRequestPayload, OciError> {
    let mut fields = selected.clone();
    fields.insert(
        String::from("apiFormat"),
        Json::Text(OciVendor::Generic.api_format().to_owned()),
    );
    fields.insert(
        String::from("messages"),
        json_of(&adapt_generic_messages(messages)?)?,
    );
    typed_request(fields, "OCIChatRequestPayload")
}

fn typed_request<T>(fields: Object, type_name: &str) -> Result<T, OciError>
where
    T: DeserializeOwned,
{
    match serde_json::to_vec(&Json::Object(fields)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| OciError {
            status: 400,
            message: format!("the OCI chat request does not fit {type_name}: {source}"),
        }),
        Err(source) => Err(encode_error(&source)),
    }
}

fn json_of<T>(value: &T) -> Result<Json, OciError>
where
    T: Serialize,
{
    match serde_json::to_vec(value) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| encode_error(&source)),
        Err(source) => Err(encode_error(&source)),
    }
}

fn encode_error(source: &serde_json::Error) -> OciError {
    OciError {
        status: 500,
        message: format!("the OCI chat request cannot be encoded as JSON: {source}"),
    }
}

fn normalize_tool_choice(selected: &mut Object) -> Result<(), OciError> {
    let Some(choice) = selected.get("toolChoice").cloned() else {
        return Ok(());
    };
    let normalized = match choice {
        Json::Null => return Ok(()),
        Json::Text(text) => tool_choice_from_string(&text),
        Json::Object(fields) => tool_choice_from_object(&fields)?,
        other => {
            return Err(OciError {
                status: 400,
                message: format!(
                    "Invalid tool_choice for OCI: expected str or dict, got {}",
                    json_kind(&other)
                ),
            });
        }
    };
    selected.insert(String::from("toolChoice"), normalized);
    Ok(())
}

fn tool_choice_from_string(text: &str) -> Json {
    let kind = match text.to_lowercase().as_str() {
        "auto" => Some("AUTO"),
        "none" => Some("NONE"),
        "required" | "any" => Some("REQUIRED"),
        _ => None,
    };
    match kind {
        Some(kind) => Json::Object(Object::from_iter([(
            String::from("type"),
            Json::Text(kind.to_owned()),
        )])),
        None => Json::Object(Object::from_iter([
            (String::from("type"), Json::Text(String::from("FUNCTION"))),
            (String::from("name"), Json::Text(text.to_owned())),
        ])),
    }
}

fn tool_choice_from_object(fields: &Object) -> Result<Json, OciError> {
    let Some(Json::Text(raw_type)) = fields.get("type") else {
        return Err(OciError {
            status: 400,
            message: format!(
                "Invalid tool_choice for OCI: missing or non-string 'type' in {}",
                Json::Object(fields.clone())
            ),
        });
    };
    let upper = raw_type.to_uppercase();
    if upper == "FUNCTION" {
        return function_tool_choice(fields);
    }
    if OK_TOOL_CHOICE_TYPES.contains(&upper.as_str()) {
        return Ok(Json::Object(Object::from_iter([(
            String::from("type"),
            Json::Text(upper),
        )])));
    }
    Err(OciError {
        status: 400,
        message: format!(
            "Invalid tool_choice for OCI: unsupported type {raw_type}; expected one of 'FUNCTION', 'AUTO', 'NONE', 'REQUIRED'"
        ),
    })
}

fn function_tool_choice(fields: &Object) -> Result<Json, OciError> {
    let found = match fields.get("function") {
        Some(Json::Object(function)) => function.get("name"),
        _ => fields.get("name"),
    };
    let name = match found {
        Some(Json::Text(name)) if !name.is_empty() => name.clone(),
        _ => {
            return Err(OciError {
                status: 400,
                message: MISSING_FUNCTION_NAME.to_owned(),
            });
        }
    };
    Ok(Json::Object(Object::from_iter([
        (String::from("type"), Json::Text(String::from("FUNCTION"))),
        (String::from("name"), Json::Text(name)),
    ])))
}

const fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(_) => "number",
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}

fn normalize_response_format(selected: &mut Object, vendor: OciVendor) -> Result<(), OciError> {
    let Some(Json::Object(format)) = selected.get("responseFormat") else {
        return Ok(());
    };
    let Some(type_value) = format.get("type") else {
        return Ok(());
    };
    let Json::Text(raw_type) = type_value else {
        return Err(OciError {
            status: 400,
            message: String::from("response_format type must be a string"),
        });
    };
    let replacement = response_format_value(format, raw_type, vendor)?;
    selected.insert(String::from("responseFormat"), replacement);
    Ok(())
}

fn response_format_value(
    format: &Object,
    raw_type: &str,
    vendor: OciVendor,
) -> Result<Json, OciError> {
    let format_type = raw_type.to_lowercase();
    if format_type == "text" {
        return Ok(type_only("TEXT"));
    }
    if vendor == OciVendor::Cohere {
        return Ok(cohere_response_format(format));
    }
    if format_type == "json_schema" {
        return json_schema_format(format);
    }
    let upper = format_type.to_uppercase();
    let kind = if upper == "JSON" {
        String::from("JSON_OBJECT")
    } else {
        upper
    };
    Ok(type_only(&kind))
}

fn type_only(kind: &str) -> Json {
    Json::Object(Object::from_iter([(
        String::from("type"),
        Json::Text(kind.to_owned()),
    )]))
}

fn cohere_response_format(format: &Object) -> Json {
    let mut payload = Object::from_iter([(
        String::from("type"),
        Json::Text(String::from("JSON_OBJECT")),
    )]);
    if let Some(schema) = format
        .get("json_schema")
        .and_then(Json::as_object)
        .and_then(|fields| fields.get("schema"))
        .filter(|value| !matches!(value, Json::Null))
    {
        payload.insert(String::from("schema"), schema.clone());
    }
    Json::Object(payload)
}

fn json_schema_format(format: &Object) -> Result<Json, OciError> {
    let Some(json_schema) = format.get("json_schema").and_then(Json::as_object) else {
        return Err(OciError {
            status: 400,
            message: String::from(
                "response_format type 'json_schema' requires a 'json_schema' object",
            ),
        });
    };
    Ok(Json::Object(retained_json_schema(json_schema)))
}

fn retained_json_schema(json_schema: &Object) -> Object {
    let mut retained = Object::new();
    match json_schema.get("name") {
        Some(name) if !matches!(name, Json::Null) => {
            retained.insert(String::from("name"), name.clone());
        }
        _ => {
            retained.insert(String::from("name"), Json::Text(String::from("response")));
        }
    }
    if let Some(description) = json_schema
        .get("description")
        .filter(|value| !matches!(value, Json::Null))
    {
        retained.insert(String::from("description"), description.clone());
    }
    if let Some(schema) = json_schema
        .get("schema")
        .filter(|value| !matches!(value, Json::Null))
    {
        retained.insert(String::from("schema"), schema.clone());
    }
    if let Some(strict) = json_schema
        .get("strict")
        .filter(|value| !matches!(value, Json::Null))
    {
        retained.insert(String::from("isStrict"), strict.clone());
    }
    Object::from_iter([
        (
            String::from("type"),
            Json::Text(String::from("JSON_SCHEMA")),
        ),
        (String::from("jsonSchema"), Json::Object(retained)),
    ])
}
