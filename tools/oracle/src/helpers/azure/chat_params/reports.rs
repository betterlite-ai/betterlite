use betterlite_azure::chat_params::capabilities::AzureCapabilities;
use betterlite_azure::chat_params::{gpt5, o_series};
use betterlite_openai::openai_base_config::{
    ModelFamily, Object as ParamsObject, azure_model_family,
};
use betterlite_openai::reasoning_models::gpt_5::Mapping as Gpt5Mapping;
use betterlite_openai::reasoning_models::{DropParams, effort_resolves_to_none};
use betterlite_prompts::arguments::json::Object;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::fixture::MapFixture;
use super::input::{
    AliasCall, AllowlistCall, MappingCall, StreamCall, TransformCall, TransformStep,
};
use crate::helpers::HelperError;

pub(super) fn runtime() -> std::io::Result<Runtime> {
    Builder::new_current_thread().build()
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
    call.steps
        .iter()
        .map(|step| {
            let mut fixture = MapFixture::new(&call.install);
            let catalog = fixture.catalog.clone();
            let supported =
                runtime.block_on(gpt5::supported_params(&mut fixture, &step.model, &catalog));
            let mut azure = AzureCapabilities {
                inner: &mut fixture,
            };
            let resolves_none =
                runtime.block_on(effort_resolves_to_none(&mut azure, &step.model, None));
            json!({
                "model": &step.model,
                "family": matches!(azure_model_family(&step.model), ModelFamily::GptReasoning),
                "supported": supported,
                "resolves_none": resolves_none,
            })
        })
        .collect()
}

pub(super) fn stream_steps(runtime: &Runtime, call: &StreamCall) -> Vec<Value> {
    call.steps
        .iter()
        .map(|step| {
            let mut fixture = MapFixture::new(&call.install);
            let fake = runtime.block_on(o_series::should_fake_stream(
                &mut fixture,
                step.model.as_deref(),
                step.stream,
                step.provider.as_deref(),
            ));
            json!({"model": step.model, "stream": step.stream, "fake": fake})
        })
        .collect()
}

pub(super) fn gpt5_models(runtime: &Runtime, call: &AllowlistCall) -> Vec<Value> {
    call.models
        .iter()
        .map(|model| {
            let mut fixture = MapFixture::new(&call.install);
            let catalog = fixture.catalog.clone();
            let supported = runtime.block_on(gpt5::supported_params(&mut fixture, model, &catalog));
            json!({"model": model, "supported": supported})
        })
        .collect()
}

pub(super) fn o_series_models(runtime: &Runtime, call: &AllowlistCall) -> Vec<Value> {
    call.models
        .iter()
        .map(|model| {
            let mut fixture = MapFixture::new(&call.install);
            let catalog = fixture.catalog.clone();
            let supported =
                runtime.block_on(o_series::supported_params(&mut fixture, model, &catalog));
            json!({"model": model, "supported": supported})
        })
        .collect()
}

pub(super) fn mapping_steps(runtime: &Runtime, call: &MappingCall) -> Vec<Value> {
    call.steps
        .iter()
        .map(|step| {
            let mut fixture = MapFixture::new(&call.install);
            let catalog = fixture.catalog.clone();
            let mut requested = step.non_default.clone();
            let mut existing = step.optional.clone();
            let mapped = {
                let mut mapping = Gpt5Mapping {
                    model: step.model.as_str(),
                    non_default_params: &mut requested,
                    optional_params: &mut existing,
                    drop_params: DropParams {
                        global: step.global_drop_params,
                        request: step.drop_params,
                    },
                    catalog: &catalog,
                };
                runtime.block_on(gpt5::map_params(&mut fixture, &mut mapping))
            };
            match mapped {
                Ok(()) => json!({"ok": true, "value": existing, "status": Value::Null}),
                Err(error) => {
                    json!({"ok": false, "value": Value::Null, "status": error.status_code()})
                }
            }
        })
        .collect()
}

pub(super) fn transform_steps(
    runtime: &Runtime,
    call: &TransformCall,
) -> Result<Vec<Value>, Box<HelperError>> {
    call.steps
        .iter()
        .map(|step| transform_step(runtime, call, step))
        .collect()
}

fn transform_step(
    runtime: &Runtime,
    call: &TransformCall,
    step: &TransformStep,
) -> Result<Value, Box<HelperError>> {
    let mut fixture = MapFixture::new(&call.install);
    let catalog = fixture.catalog.clone();
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
    match gpt5::transform_request(
        &step.model,
        &step.messages,
        &optional,
        call.install.openai_system_messages_first,
    ) {
        Ok(body) => Ok(json!({"ok": true, "value": projected(&body), "status": Value::Null})),
        Err(error) => Ok(json!({"ok": false, "value": Value::Null, "status": error.status_code()})),
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
