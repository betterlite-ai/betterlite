use std::collections::BTreeMap;

use betterlite_azure::azure_chat_base_policy::auth::{
    EU_REGIONS, SPECIAL_AUTH_PARAMS, US_REGIONS, map_special_auth_params,
};
use betterlite_azure::azure_chat_base_policy::errors::provider_error;
use betterlite_azure::azure_chat_base_policy::mapping::{
    MapPolicy, format_supported_for_version, map_openai_params, supports_response_format,
};
use betterlite_azure::azure_chat_base_policy::messages::transform_request;
use betterlite_config::loader::ConfigLoader;
use betterlite_openai::base_helpers::provider_config;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::{Value, json};

use crate::helpers::HelperError;

use super::TRANSFORM_MODEL;
use super::input::{
    ClassStep, ErrorStep, Loaded, MapStep, ModelCall, Settings, SpecialAuthCall, TransformStep,
    VersionCall,
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

fn step_result(result: Result<Object, Option<u16>>) -> Value {
    match result {
        Ok(value) => json!({"ok": true, "value": value, "status": null}),
        Err(status) => json!({"ok": false, "value": null, "status": status}),
    }
}

fn step_policy(step: &MapStep, loaded: Loaded) -> MapPolicy<'_> {
    MapPolicy {
        model: &step.model,
        api_version: &step.api_version,
        drop_params: step.drop_params,
        global_drop_params: step.global_drop_params.unwrap_or(loaded.drop_params),
    }
}

fn map_step(step: &MapStep, loaded: Loaded) -> Value {
    let mut optional = step.optional.clone();
    let policy = step_policy(step, loaded);
    let outcome = map_openai_params(&step.non_default, &mut optional, &policy);
    step_result(
        outcome
            .map(|()| optional)
            .map_err(|error| error.status_code()),
    )
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
    let outcome = transform_request(
        TRANSFORM_MODEL,
        &step.messages,
        &step.optional,
        system_first,
    );
    step_result(outcome.map_err(|error| error.status_code()))
}

pub(super) fn transform_steps(steps: &[TransformStep], loaded: Loaded) -> Value {
    let mut reports = Vec::with_capacity(steps.len());
    for step in steps {
        reports.push(transform_step(step, loaded));
    }
    Value::Array(reports)
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

fn model_entries(models: &[String], entry: fn(&str) -> Value) -> Vec<Value> {
    let mut entries = Vec::with_capacity(models.len());
    for model in models {
        entries.push(entry(model));
    }
    entries
}

pub(super) fn model_report(call: &ModelCall, loaded: Loaded, entry: fn(&str) -> Value) -> Value {
    json!({
        "models": model_entries(&call.models, entry),
        "steps": mapping_steps(&call.steps, loaded),
    })
}

pub(super) fn version_reports(call: &VersionCall, loaded: Loaded) -> Value {
    let models = model_entries(
        &call.models,
        |model| json!({"model": model, "supported": supports_response_format(model)}),
    );
    let mut versions = Vec::with_capacity(call.versions.len());
    for pair in &call.versions {
        versions.push(version_report(&pair.year, &pair.month));
    }
    json!({"models": models, "versions": versions, "steps": mapping_steps(&call.steps, loaded)})
}
