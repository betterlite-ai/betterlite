use std::collections::BTreeMap;
use std::error::Error;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::copilot_chat::Settings;
use betterlite_config::sections::vertex_schema_shaping::Raw;
use betterlite_core::api::copilot_responses::input::{LiteLlmParams, validated_input};
use betterlite_core::api::copilot_responses::model_gate::supports_responses_api;
use betterlite_core::api::copilot_responses::stream::StreamItemIds;
use betterlite_core::api::copilot_responses::{ResponsesConfig, SUPPORTS_NATIVE_WEBSOCKET};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_openai::response_events::event_decode::{
    DecodedStreamResponse, transform_streaming_response,
};
use betterlite_openai::response_input::ResponsesInput;
use fake::{CatalogInfo, StoredKey};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};
use uuid::Uuid;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod fake;
mod tests;

const MODEL: &str = "gpt-5";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    settings: BTreeMap<String, String>,
    steps: Vec<Step>,
}

// a typed params object: model_info and the other fields it may carry are never read, so only input is modeled
#[derive(Debug, Deserialize)]
pub(crate) struct Params {
    input: Option<ResponsesInput>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Supports {
        model: String,
        models: BTreeMap<String, Value>,
    },
    Reasoning {
        input: ResponsesInput,
    },
    Stream {
        events: Vec<IndexMap<String, Json>>,
    },
    Auth {
        copilot_key: Option<String>,
        headers: BTreeMap<String, String>,
        params: Option<Params>,
    },
    Url {
        endpoints_api: Option<String>,
        api_base: Option<String>,
        env_api_base: Option<String>,
    },
    Capability,
}

pub(crate) struct Op;

fn failure(error: &dyn Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "CopilotResponses".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn config(
    depth: i64,
    token: Option<&String>,
    endpoints_api: Option<&String>,
    env_api_base: Option<&String>,
) -> ResponsesConfig<StoredKey> {
    ResponsesConfig {
        credentials: StoredKey {
            token: token.cloned(),
            base: endpoints_api.cloned(),
        },
        settings: Settings {
            disable_system_to_assistant: false,
            api_base: env_api_base.cloned(),
        },
        max_recurse_depth: depth,
        item_ids: StreamItemIds::default(),
    }
}

// the class and the type of the event matter less than where its ids ended up, so only those are compared
fn projected(decoded: &DecodedStreamResponse) -> Result<Value, Box<HelperError>> {
    let event = match decoded {
        DecodedStreamResponse::Known { record, .. } => serde_json::to_value(record),
        DecodedStreamResponse::Generic(generic) => serde_json::to_value(generic),
        DecodedStreamResponse::Lax { fields, .. } => serde_json::to_value(fields),
    }
    .map_err(|error| failure(&error))?;
    let nested = event.get("item").and_then(|item| item.get("id"));
    Ok(json!({
        "item_id": event.get("item_id").and_then(Value::as_str),
        "nested_id": nested.and_then(Value::as_str),
    }))
}

fn auth_value(
    runtime: &Runtime,
    mut responses: ResponsesConfig<StoredKey>,
    headers: &BTreeMap<String, String>,
    params: Option<&Params>,
) -> Value {
    let typed = LiteLlmParams::Typed {
        input: params.and_then(|given| given.input.as_ref()),
    };
    let outcome = runtime.block_on(responses.validate_environment(
        headers.clone(),
        &typed,
        MODEL,
        Uuid::from_u128(1),
    ));
    match outcome {
        Ok(merged) => {
            let request_id = merged.contains_key("x-request-id");
            let shown: BTreeMap<String, String> = merged
                .into_iter()
                .filter(|(name, _value)| name != "x-request-id")
                .collect();
            json!({"headers": shown, "request_id": request_id})
        }
        Err(error) => json!({"raised": error.status()}),
    }
}

fn supports_value(
    runtime: &Runtime,
    model: &str,
    models: &BTreeMap<String, Value>,
) -> Result<Value, Box<HelperError>> {
    let text = serde_json::to_string(models).map_err(|error| failure(&error))?;
    let catalog = ModelCatalog::parse(text.as_bytes(), UnknownFields::Warn)
        .map_err(|error| failure(&error))?;
    let mut source = CatalogInfo { catalog: &catalog };
    Ok(Value::Bool(runtime.block_on(supports_responses_api(
        model,
        &mut source,
        &catalog,
    ))))
}

fn step_value(runtime: &Runtime, depth: i64, step: &Step) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Supports { model, models } => supports_value(runtime, model, models),
        Step::Reasoning { input: items } => {
            serde_json::to_value(validated_input(items)).map_err(|error| failure(&error))
        }
        Step::Stream { events } => {
            let mut ids = StreamItemIds::default();
            let mut results = Vec::new();
            for event in events {
                let decoded = transform_streaming_response(&ids.normalize(event.clone()));
                results.push(projected(&decoded)?);
            }
            Ok(Value::Array(results))
        }
        Step::Auth {
            copilot_key,
            headers,
            params,
        } => Ok(auth_value(
            runtime,
            config(depth, copilot_key.as_ref(), None, None),
            headers,
            params.as_ref(),
        )),
        Step::Url {
            endpoints_api,
            api_base,
            env_api_base,
        } => {
            let mut responses = config(depth, None, endpoints_api.as_ref(), env_api_base.as_ref());
            Ok(Value::String(
                runtime.block_on(responses.get_complete_url(api_base.as_deref())),
            ))
        }
        Step::Capability => Ok(json!({"native_websocket": SUPPORTS_NATIVE_WEBSOCKET})),
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let depth = Raw::default()
            .convert(&input.settings)
            .or_config_error()?
            .max_recurse_depth;
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure(&error))?;
        let mut results = Vec::new();
        for step in &input.steps {
            results.push(step_value(&runtime, depth, step)?);
        }
        Ok(Value::Array(results))
    }
}
