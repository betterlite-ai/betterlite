use betterlite_azure::chat_params::capabilities::AzureCapabilities;
use betterlite_azure::chat_params::error::ChatParamsError;
use betterlite_azure::chat_params::{gpt5, o_series};
use betterlite_context::chat_config_selection::azure::is_o_series_model;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ModelFamily, Object as ParamsObject, azure_model_family,
};
use betterlite_openai::reasoning_models::gpt_5::Mapping as Gpt5Mapping;
use betterlite_openai::reasoning_models::{DropParams, effort_resolves_to_none};
use betterlite_prompts::arguments::json::Object;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::fixture::MapFixture;
use super::input::{
    AliasCall, AliasStep, AllowlistCall, Install, MapStep, MappingCall, StreamCall, StreamStep,
    TransformCall, TransformStep,
};
use crate::helpers::HelperError;

#[derive(Clone, Copy)]
pub(super) enum Series {
    Gpt5,
    OSeries,
}

pub(super) fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn alias_models(runtime: &Runtime, call: &AliasCall) -> Vec<Value> {
    let mut models = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        models.push(alias_row(runtime, &call.install, step));
    }
    models
}

fn fixture_with_catalog(install: &Install) -> (MapFixture, CatalogCapabilities) {
    let fixture = MapFixture::new(install);
    let catalog = fixture.catalog.clone();
    (fixture, catalog)
}

fn alias_row(runtime: &Runtime, install: &Install, step: &AliasStep) -> Value {
    let (mut fixture, catalog) = fixture_with_catalog(install);
    let supported = runtime.block_on(gpt5::supported_params(&mut fixture, &step.model, &catalog));
    let resolves_none = runtime.block_on(effort_resolves_to_none(
        &mut AzureCapabilities {
            inner: &mut fixture,
        },
        &step.model,
        None,
    ));
    let family = azure_model_family(&step.model);
    json!({
        "model": &step.model,
        "family": matches!(family, ModelFamily::GptReasoning),
        "supported": supported,
        "resolves_none": resolves_none,
    })
}

pub(super) fn stream_steps(runtime: &Runtime, call: &StreamCall) -> Vec<Value> {
    let mut steps = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        steps.push(stream_row(runtime, &call.install, step));
    }
    steps
}

fn stream_row(runtime: &Runtime, install: &Install, step: &StreamStep) -> Value {
    let mut fixture = MapFixture::new(install);
    let fake = fake_answer(runtime, &mut fixture, step);
    json!({"model": step.model, "stream": step.stream, "fake": fake})
}

fn fake_answer(runtime: &Runtime, fixture: &mut MapFixture, step: &StreamStep) -> bool {
    runtime.block_on(o_series::should_fake_stream(
        fixture,
        step.model.as_deref(),
        step.stream,
        step.provider.as_deref(),
    ))
}

pub(super) fn allowlist_models(
    runtime: &Runtime,
    call: &AllowlistCall,
    series: Series,
) -> Vec<Value> {
    let mut models = Vec::with_capacity(call.models.len());
    for model in &call.models {
        models.push(allowlist_row(runtime, &call.install, series, model));
    }
    models
}

fn allowlist_row(runtime: &Runtime, install: &Install, series: Series, model: &str) -> Value {
    let (mut fixture, catalog) = fixture_with_catalog(install);
    let supported = match series {
        Series::Gpt5 => runtime.block_on(gpt5::supported_params(&mut fixture, model, &catalog)),
        Series::OSeries => {
            runtime.block_on(o_series::supported_params(&mut fixture, model, &catalog))
        }
    };
    json!({"model": model, "supported": supported})
}

pub(super) fn mapping_steps(runtime: &Runtime, call: &MappingCall) -> Vec<Value> {
    let mut steps = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        steps.push(mapping_row(runtime, &call.install, step));
    }
    steps
}

fn mapping_row(runtime: &Runtime, install: &Install, step: &MapStep) -> Value {
    let (mut fixture, catalog) = fixture_with_catalog(install);
    let (mapped, existing) = apply_mapping(runtime, &mut fixture, &catalog, step);
    mapping_value(mapped, existing)
}

fn mapping_value(mapped: Result<(), ChatParamsError>, existing: ParamsObject) -> Value {
    match mapped {
        Ok(()) => json!({"ok": true, "value": existing, "status": Value::Null}),
        Err(error) => json!({"ok": false, "value": Value::Null, "status": error.status_code()}),
    }
}

fn apply_mapping(
    runtime: &Runtime,
    fixture: &mut MapFixture,
    catalog: &CatalogCapabilities,
    step: &MapStep,
) -> (Result<(), ChatParamsError>, ParamsObject) {
    let mut requested = step.non_default.clone();
    let mut existing = step.optional.clone();
    let mut mapping = Gpt5Mapping {
        model: step.model.as_str(),
        non_default_params: &mut requested,
        optional_params: &mut existing,
        drop_params: DropParams {
            global: step.global_drop_params,
            request: step.drop_params,
        },
        catalog,
    };
    let mapped = runtime.block_on(gpt5::map_params(fixture, &mut mapping));
    (mapped, existing)
}

pub(super) fn transform_steps(
    runtime: &Runtime,
    call: &TransformCall,
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut steps = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        steps.push(transform_step(runtime, call, step)?);
    }
    Ok(steps)
}

fn transform_step(
    runtime: &Runtime,
    call: &TransformCall,
    step: &TransformStep,
) -> Result<Value, Box<HelperError>> {
    let (mut fixture, catalog) = fixture_with_catalog(&call.install);
    let mut requested = requested_params(step);
    let mut existing = ParamsObject::new();
    let mapped = {
        let mut mapping = Gpt5Mapping {
            model: step.model.as_str(),
            non_default_params: &mut requested,
            optional_params: &mut existing,
            drop_params: DropParams {
                global: call.install.drop_params,
                request: false,
            },
            catalog: &catalog,
        };
        runtime.block_on(gpt5::map_params(&mut fixture, &mut mapping))
    };
    if let Err(error) = mapped {
        return Ok(json!({"ok": false, "value": Value::Null, "status": error.status_code()}));
    }
    let optional: Object = serde_json::from_value(Value::Object(existing))
        .map_err(|error| invalid_input(error.to_string()))?;
    let transformed = transformed_body(runtime, &mut fixture, call, step, &optional);
    match transformed {
        Ok(body) => Ok(json!({"ok": true, "value": projected(&body), "status": Value::Null})),
        Err(error) => Ok(json!({"ok": false, "value": Value::Null, "status": error.status_code()})),
    }
}

fn requested_params(step: &TransformStep) -> ParamsObject {
    let mut requested = ParamsObject::new();
    if let Some(tools) = &step.tools {
        requested.insert(String::from("tools"), tools.clone());
    }
    if let Some(choice) = &step.tool_choice {
        requested.insert(String::from("tool_choice"), Value::String(choice.clone()));
    }
    if let Some(effort) = &step.reasoning_effort {
        requested.insert(
            String::from("reasoning_effort"),
            Value::String(effort.clone()),
        );
    }
    requested
}

fn transformed_body(
    runtime: &Runtime,
    fixture: &mut MapFixture,
    call: &TransformCall,
    step: &TransformStep,
    optional: &Object,
) -> Result<Object, ChatParamsError> {
    if is_o_series_model(&step.model) {
        runtime.block_on(o_series::transform_request(
            fixture,
            &step.model,
            &step.messages,
            optional,
            call.install.openai_system_messages_first,
        ))
    } else {
        gpt5::transform_request(
            &step.model,
            &step.messages,
            optional,
            call.install.openai_system_messages_first,
        )
    }
}

fn projected(body: &Object) -> Object {
    const PROJECTED: [&str; 5] = [
        "model",
        "messages",
        "tools",
        "tool_choice",
        "reasoning_effort",
    ];
    let mut projected = Object::new();
    for name in PROJECTED {
        if let Some(value) = body.get(name) {
            projected.insert(String::from(name), value.clone());
        }
    }
    projected
}
