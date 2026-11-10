use std::collections::BTreeMap;

use betterlite_azure::azure_chat_base_policy::SUPPORTED_OPENAI_PARAMS;
use betterlite_azure::azure_chat_base_policy::auth::{
    EU_REGIONS, SPECIAL_AUTH_PARAMS, US_REGIONS, map_special_auth_params,
};
use betterlite_azure::azure_chat_base_policy::errors::provider_error;
use betterlite_azure::azure_chat_base_policy::mapping::{
    MapPolicy, format_supported_for_version, map_openai_params, requires_max_completion_tokens,
    supports_response_format,
};
use betterlite_azure::azure_chat_base_policy::messages::transform_request;
use betterlite_config::loader::ConfigLoader;
use betterlite_openai::base_helpers::provider_config;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::{Value, json};

use crate::helpers::HelperError;

use super::TRANSFORM_MODEL;
use super::input::{
    ClassStep, ErrorStep, Loaded, MapStep, PredicateCall, Settings, SpecialAuthCall, SupportedCall,
    TransformStep, VersionCall,
};

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

pub(super) fn load_settings(settings: Settings) -> Result<Loaded, Box<HelperError>> {
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

pub(super) fn mapping_steps(steps: &[MapStep], loaded: Loaded) -> Value {
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

pub(super) fn transform_steps(steps: &[TransformStep], loaded: Loaded) -> Value {
    Value::Array(
        steps
            .iter()
            .map(|step| transform_step(step, loaded))
            .collect(),
    )
}

pub(super) fn class_report(step: &ClassStep) -> Result<Value, Box<HelperError>> {
    let projected = provider_config(&step.values)
        .map_err(|error| helper_error("ConfigError", error.to_string()))?;
    serde_json::to_value(projected).map_err(|error| helper_error("EncodeError", error.to_string()))
}

pub(super) fn auth_report(call: &SpecialAuthCall) -> Value {
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

pub(super) fn error_report(step: &ErrorStep) -> Value {
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

pub(super) fn predicate_report(call: &PredicateCall, loaded: Loaded) -> Value {
    let flags: Vec<Value> = call
        .models
        .iter()
        .map(|model| json!({"model": model, "requires": requires_max_completion_tokens(model)}))
        .collect();
    json!({"requires": flags, "steps": mapping_steps(&call.steps, loaded)})
}

pub(super) fn supported_report(call: &SupportedCall, loaded: Loaded) -> Value {
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

pub(super) fn version_reports(call: &VersionCall, loaded: Loaded) -> Value {
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
