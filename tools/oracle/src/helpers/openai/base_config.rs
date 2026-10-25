use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, OpenAiConfig, is_azure_gpt5_model, is_gpt_audio_model,
    is_gpt_reasoning_series_name, is_o_series_model, metadata_filter_limit,
    overlapping_api_base_path,
};
use serde_json::{Map, Value, json};

use crate::helpers::{HelperError, HelperResult};
use betterlite_openai::openai_base_config::forwarding::{OpenAiErrorParts, OpenAiErrorRecord};

type Object = Map<String, Value>;

mod tests;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BaseCall {
    OSeriesRouting(Object),
    Gpt5Routing(Object),
    AudioRouting(Object),
    CatalogUserOnly(Object),
    OldGptResponseFormatDrop(Object),
    UnknownReasoningEffort(Object),
    BaseParameterAllowlist(Object),
    BaseAuthOverride(Object),
    BaseProjectionForwarding(Object),
    BaseRequestOverlay(Object),
    OpenaiErrorDefaults(Object),
    MetadataFilterLimit(Object),
    OverlappingApiBasePath(Object),
    AzureGpt5RoutePredicate(Object),
}
fn invalid_input(message: String) -> HelperError {
    let class = String::from("InvalidInput");
    HelperError {
        class,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}
fn text<'input>(fields: &'input Object, name: &str) -> Result<&'input str, Box<HelperError>> {
    let value = fields.get(name).and_then(Value::as_str);
    value.map_or_else(
        || Err(Box::new(invalid_input(format!("{name} must be a string")))),
        Ok,
    )
}
fn catalog(fields: &Object) -> CatalogCapabilities {
    let set = |name| {
        fields
            .get(name)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    CatalogCapabilities::new(set("chat_models"), set("text_models"))
}
fn configuration(fields: &Object) -> OpenAiConfig {
    let family = match fields.get("family").and_then(Value::as_str) {
        Some("azure_ai_studio") => ConfigFamily::AzureAiStudio,
        Some("other") => ConfigFamily::Other,
        _ => ConfigFamily::OpenAi,
    };
    let key = fields
        .get("api_key")
        .and_then(Value::as_str)
        .map(secrecy::SecretString::from);
    let headers = fields
        .get("headers")
        .and_then(Value::as_object)
        .map(|values| {
            values
                .iter()
                .filter_map(|(name, value)| {
                    value.as_str().map(|text| (name.clone(), text.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    OpenAiConfig::new(family, key, headers, Map::new())
}
fn supported(fields: &Object) -> Value {
    let config = configuration(fields);
    let model = fields
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    json!(config.supported_params(model, &catalog(fields)))
}
fn object(fields: &Object, name: &str) -> Object {
    let value = fields.get(name).and_then(Value::as_object);
    value.map_or_else(Map::new, Clone::clone)
}
fn mapped(fields: &Object) -> Value {
    let config = configuration(fields);
    let mut existing = object(fields, "existing");
    let requested = object(fields, "requested");
    config.map_params(
        fields
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        &requested,
        &mut existing,
        &catalog(fields),
    );
    Value::Object(existing)
}
fn request(fields: &Object, model: &str) -> Value {
    let messages = fields.get("messages").cloned().unwrap_or(Value::Null);
    let optional = fields
        .get("optional")
        .and_then(Value::as_object)
        .map_or_else(Map::new, Clone::clone);
    json!(configuration(fields).request_body(model, messages, &optional,))
}
fn error(status_code: u16, message: String) -> Result<Value, Box<HelperError>> {
    let record = OpenAiErrorRecord::from_parts(OpenAiErrorParts {
        status_code,
        message,
        headers: None,
        request: None,
        response: None,
        body: None,
    });
    serde_json::to_value(record).map_err(|error| Box::new(invalid_input(error.to_string())))
}
fn metadata(fields: &Object) -> Result<Value, Box<HelperError>> {
    let value = metadata_filter_limit(fields.get("metadata").and_then(Value::as_object));
    serde_json::to_value(value).map_err(|error| Box::new(invalid_input(error.to_string())))
}
fn openai_error(fields: &Object) -> Result<Value, Box<HelperError>> {
    let status =
        fields
            .get("status_code")
            .and_then(Value::as_u64)
            .map_or(Ok(500), |raw_status| {
                u16::try_from(raw_status)
                    .map_err(|_| Box::new(invalid_input(String::from("status too large"))))
            })?;
    error(status, text(fields, "message")?.to_owned())
}
fn overlap(fields: &Object) -> Result<Value, Box<HelperError>> {
    let api_base = text(fields, "api_base")?;
    let ending_path = text(fields, "ending_path")?;
    overlapping_api_base_path(api_base, ending_path)
        .map(Value::String)
        .map_err(|error| Box::new(invalid_input(error.to_string())))
}
fn invoke(input: &BaseCall) -> Result<Value, Box<HelperError>> {
    match input {
        BaseCall::OSeriesRouting(fields) => Ok(json!(is_o_series_model(
            text(fields, "model")?,
            &catalog(fields)
        ))),
        BaseCall::Gpt5Routing(fields) => {
            Ok(json!(is_gpt_reasoning_series_name(text(fields, "model",)?)))
        }
        BaseCall::AudioRouting(fields) => Ok(json!(is_gpt_audio_model(
            text(fields, "model")?,
            &catalog(fields)
        ))),
        BaseCall::CatalogUserOnly(fields)
        | BaseCall::OldGptResponseFormatDrop(fields)
        | BaseCall::UnknownReasoningEffort(fields) => Ok(supported(fields)),
        BaseCall::BaseParameterAllowlist(fields) => Ok(mapped(fields)),
        BaseCall::BaseAuthOverride(fields) => {
            Ok(json!(configuration(fields).validate_environment()))
        }
        BaseCall::BaseProjectionForwarding(fields) => Ok(json!({
            "post_call": fields.get("raw_text"),
            "response_headers": fields.get("raw_headers"),
            "projected": fields.get("decoded")
        })),
        BaseCall::BaseRequestOverlay(fields) => Ok(request(fields, text(fields, "model")?)),
        BaseCall::OpenaiErrorDefaults(fields) => openai_error(fields),
        BaseCall::MetadataFilterLimit(fields) => metadata(fields),
        BaseCall::OverlappingApiBasePath(fields) => overlap(fields),
        BaseCall::AzureGpt5RoutePredicate(fields) => {
            Ok(json!(is_azure_gpt5_model(text(fields, "model")?)))
        }
    }
}
pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = BaseCall;
    fn run(input: &Self::Input) -> HelperResult {
        invoke(input)
    }
}
