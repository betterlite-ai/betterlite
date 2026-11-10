use std::collections::BTreeMap;

use betterlite_azure::azure_chat_base_policy::auth::{
    EU_REGIONS, SPECIAL_AUTH_PARAMS, US_REGIONS, map_special_auth_params,
};
use betterlite_azure::azure_chat_base_policy::errors::provider_error;
use betterlite_azure::azure_chat_base_policy::mapping::{
    MapPolicy, format_supported_for_version, map_openai_params, requires_max_completion_tokens,
    supports_response_format,
};
use betterlite_azure::azure_chat_base_policy::messages::{
    sanitized_tools_update, transform_request,
};
use betterlite_azure::azure_chat_base_policy::{AzureChatSettings, SUPPORTED_OPENAI_PARAMS};
use betterlite_config::loader::ConfigLoader;
use betterlite_openai::base_helpers::provider_config;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

const TRANSFORM_MODEL: &str = "gpt-4o";

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Input {
    ApiVersionResponseFormat(VersionCall),
    MaxCompletionTokens(PredicateCall),
    UnsupportedParam(SupportedCall),
    AzureToolChoiceDateGate(MappingCall),
    AzureRequiredToolChoice(MappingCall),
    AzureParameterShapes(MappingCall),
    AzureMessageContent(TransformCall),
    SanitizedToolSchema(SanitizeCall),
    AzureToolChoiceWithoutTools(TransformCall),
    AzureClassDefaults(ClassCall),
    AzureSpecialAuthRegions(SpecialAuthCall),
    AzureErrorFactory(ErrorFactoryCall),
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Settings {
    drop_params: Option<bool>,
    openai_system_messages_first: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Loaded {
    drop_params: bool,
    openai_system_messages_first: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MappingCall {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    steps: Vec<MapStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapStep {
    model: String,
    #[serde(default)]
    api_version: String,
    #[serde(default)]
    drop_params: bool,
    #[serde(default)]
    global_drop_params: Option<bool>,
    #[serde(default)]
    non_default: Object,
    #[serde(default)]
    optional: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VersionCall {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    versions: Vec<VersionPair>,
    #[serde(default)]
    steps: Vec<MapStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VersionPair {
    year: String,
    month: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PredicateCall {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    steps: Vec<MapStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SupportedCall {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    steps: Vec<MapStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformCall {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    steps: Vec<TransformStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformStep {
    #[serde(default)]
    messages: Vec<Message>,
    #[serde(default)]
    optional: Object,
    #[serde(default)]
    openai_system_messages_first: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SanitizeCall {
    #[serde(default)]
    optional: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassCall {
    #[serde(default)]
    steps: Vec<ClassStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassStep {
    #[serde(default)]
    values: AzureChatSettings,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpecialAuthCall {
    #[serde(default)]
    steps: Vec<AuthStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthStep {
    #[serde(default)]
    non_default: Object,
    #[serde(default)]
    optional: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorFactoryCall {
    #[serde(default)]
    steps: Vec<ErrorStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorStep {
    message: String,
    status_code: u16,
    #[serde(default)]
    headers: Option<IndexMap<String, String>>,
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn config_source(settings: Settings) -> String {
    let mut lines = Vec::new();
    if let Some(flag) = settings.drop_params {
        lines.push(format!("  drop_params: {flag}"));
    }
    if let Some(flag) = settings.openai_system_messages_first {
        lines.push(format!("  openai_system_messages_first: {flag}"));
    }
    if lines.is_empty() {
        return String::new();
    }
    format!("litellm_settings:\n{}\n", lines.join("\n"))
}

fn load_settings(settings: Settings) -> Result<Loaded, Box<HelperError>> {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&config_source(settings))
    .map_err(|error| helper_error("ConfigError", error.to_string()))?;
    Ok(Loaded {
        drop_params: config.litellm_settings.drop_params.unwrap_or(false),
        openai_system_messages_first: config
            .sections
            .openai_chat_content
            .openai_system_messages_first,
    })
}

fn map_step(step: &MapStep, loaded: Loaded) -> Value {
    let mut optional = step.optional.clone();
    let policy = MapPolicy {
        model: &step.model,
        api_version: &step.api_version,
        drop_params: step.drop_params,
        global_drop_params: step.global_drop_params.unwrap_or(loaded.drop_params),
    };
    match map_openai_params(&step.non_default, &mut optional, &policy) {
        Ok(()) => json!({"ok": true, "params": optional, "status": null}),
        Err(error) => json!({
            "ok": false,
            "params": null,
            "status": error.status_code(),
        }),
    }
}

fn mapping_steps(steps: &[MapStep], loaded: Loaded) -> Value {
    let mut reports = Vec::with_capacity(steps.len());
    for step in steps {
        reports.push(map_step(step, loaded));
    }
    Value::Array(reports)
}

fn version_report(year: &str, month: &str) -> Value {
    match format_supported_for_version(year, month) {
        Ok(supported) => json!({"ok": true, "supported": supported, "status": null}),
        Err(error) => json!({"ok": false, "supported": null, "status": error.status_code()}),
    }
}

fn transform_step(step: &TransformStep, loaded: Loaded) -> Value {
    let system_first = step
        .openai_system_messages_first
        .unwrap_or(loaded.openai_system_messages_first);
    match transform_request(
        TRANSFORM_MODEL,
        &step.messages,
        &step.optional,
        system_first,
    ) {
        Ok(body) => json!({"ok": true, "body": body, "status": null}),
        Err(error) => json!({"ok": false, "body": null, "status": error.status_code()}),
    }
}

fn transform_steps(steps: &[TransformStep], loaded: Loaded) -> Value {
    Value::Array(
        steps
            .iter()
            .map(|step| transform_step(step, loaded))
            .collect(),
    )
}

fn class_report(step: &ClassStep) -> Result<Value, Box<HelperError>> {
    let projected = provider_config(&step.values)
        .map_err(|error| helper_error("ConfigError", error.to_string()))?;
    serde_json::to_value(projected).map_err(|error| helper_error("EncodeError", error.to_string()))
}

fn auth_report(call: &SpecialAuthCall) -> Value {
    let mut steps = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        let mut optional = step.optional.clone();
        map_special_auth_params(&step.non_default, &mut optional);
        steps.push(json!(optional));
    }
    json!({
        "mapped": Object::from_iter(SPECIAL_AUTH_PARAMS.map(|(param, mapped)| (param.to_owned(), Json::Text(mapped.to_owned())))),
        "eu": EU_REGIONS,
        "us": US_REGIONS,
        "steps": steps,
    })
}

fn error_report(step: &ErrorStep) -> Value {
    let headers: Option<Vec<(String, String)>> = step.headers.as_ref().map(|headers| {
        headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    });
    let error = provider_error(&step.message, step.status_code, headers);
    json!({
        "status_code": error.status_code,
        "message": error.message,
        "headers": error.headers,
        "request": {"method": error.request.method, "url": error.request.url},
        "response": {
            "status_code": error.response.status_code,
            "request": {
                "method": error.response.request.method,
                "url": error.response.request.url,
            },
        },
        "body": error.body,
        "status_code_is_synthesized": error.status_code_is_synthesized,
    })
}

fn predicate_report(call: &PredicateCall, loaded: Loaded) -> Value {
    let flags: Vec<Value> = call
        .models
        .iter()
        .map(|model| json!({"model": model, "requires": requires_max_completion_tokens(model)}))
        .collect();
    json!({"requires": flags, "steps": mapping_steps(&call.steps, loaded)})
}

fn supported_report(call: &SupportedCall, loaded: Loaded) -> Value {
    let lists: Vec<Value> = call
        .models
        .iter()
        .map(|model| {
            json!({
                "model": model,
                "params": SUPPORTED_OPENAI_PARAMS,
            })
        })
        .collect();
    json!({"supported": lists, "steps": mapping_steps(&call.steps, loaded)})
}

fn version_reports(call: &VersionCall, loaded: Loaded) -> Value {
    let versions: Vec<Value> = call
        .versions
        .iter()
        .map(|pair| version_report(&pair.year, &pair.month))
        .collect();
    let models: Vec<Value> = call
        .models
        .iter()
        .map(|model| json!({"model": model, "supported": supports_response_format(model)}))
        .collect();
    json!({
        "model_supported": models,
        "version_supported": versions,
        "steps": mapping_steps(&call.steps, loaded),
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ApiVersionResponseFormat(call) => {
                Ok(version_reports(call, load_settings(call.settings)?))
            }
            Input::MaxCompletionTokens(call) => {
                Ok(predicate_report(call, load_settings(call.settings)?))
            }
            Input::UnsupportedParam(call) => {
                Ok(supported_report(call, load_settings(call.settings)?))
            }
            Input::AzureToolChoiceDateGate(call)
            | Input::AzureRequiredToolChoice(call)
            | Input::AzureParameterShapes(call) => Ok(json!({
                "steps": mapping_steps(&call.steps, load_settings(call.settings)?),
            })),
            Input::AzureMessageContent(call) | Input::AzureToolChoiceWithoutTools(call) => {
                Ok(json!({
                    "steps": transform_steps(&call.steps, load_settings(call.settings)?),
                }))
            }
            Input::SanitizedToolSchema(call) => Ok(json!(sanitized_tools_update(&call.optional))),
            Input::AzureClassDefaults(call) => Ok(json!({
                "steps": call.steps.iter().map(class_report).collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            })),
            Input::AzureSpecialAuthRegions(call) => Ok(auth_report(call)),
            Input::AzureErrorFactory(call) => Ok(json!({
                "steps": call.steps.iter().map(error_report).collect::<Vec<Value>>(),
            })),
        }
    }
}
