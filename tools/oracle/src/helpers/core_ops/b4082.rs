use betterlite_core::shadow_requests::Object;
use betterlite_core::shadow_requests::guardrails::post_guardrail_kwargs;
use betterlite_core::shadow_requests::sample::judgeable_sample;
use betterlite_core::shadow_requests::surfaces::{Surface, SurfaceOps, surface_ops};
use betterlite_core::shadow_requests::web_search::request_has_hosted_web_search;
use betterlite_logging::callback_logger::event::GuardrailRequestSnapshot;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json, to_value};
use tokio::runtime::{Builder, Runtime};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    call_type: String,
    #[serde(default)]
    kwargs: Object,
    #[serde(default)]
    model_parameters: Object,
    #[serde(default = "default_response")]
    response: Json,
    #[serde(default)]
    metadata: Object,
    #[serde(default)]
    snapshot_body: Object,
    #[serde(default)]
    snapshot_metadata: Object,
    #[serde(default)]
    request: Object,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    SurfaceOps,
    ChatRequest,
    Sample,
    PostGuardrailKwargs,
    HostedWebSearch,
}

const fn default_response() -> Json {
    Json::Null
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.operation {
            Operation::SurfaceOps => Ok(surface_row(&input.call_type)),
            Operation::ChatRequest => chat_request_row(input, row(&input.call_type)?),
            Operation::Sample => sample_row(input, row(&input.call_type)?),
            Operation::PostGuardrailKwargs => {
                Ok(json!({"result": guardrail_row(input, row(&input.call_type)?)}))
            }
            Operation::HostedWebSearch => {
                Ok(json!({"has": request_has_hosted_web_search(&input.request)}))
            }
        }
    }
}

fn surface_row(call_type: &str) -> Value {
    let Some(ops) = surface_ops(call_type) else {
        return json!({"found": false, "wire_params": null, "surface": null});
    };
    let surface = match ops.surface {
        Surface::Chat => "chat",
        Surface::AnthropicMessages => "anthropic_messages",
        Surface::Responses => "responses",
    };
    json!({"found": true, "wire_params": ops.wire_params, "surface": surface})
}

fn chat_request_row(input: &Input, ops: &SurfaceOps) -> HelperResult {
    let request =
        new_runtime()?.block_on((ops.chat_request)(&input.kwargs, &input.model_parameters));
    request
        .map_err(|error| failure("ValueError", error.to_string()))
        .and_then(without_nulls)
}

fn without_nulls(request: Object) -> HelperResult {
    let shaped: Object = request
        .into_iter()
        .filter(|(_, value)| !matches!(value, Json::Null))
        .collect();
    to_value(&shaped).map_err(|error| failure("ValueError", error.to_string()))
}

fn sample_row(input: &Input, ops: &SurfaceOps) -> HelperResult {
    let runtime = new_runtime()?;
    let sample = runtime.block_on(judgeable_sample(
        ops,
        &input.kwargs,
        &input.model_parameters,
        &input.response,
    ));
    let Some((messages, params, real_text)) = sample else {
        return Ok(json!({"sample": null}));
    };
    Ok(json!({
        "sample": {
            "messages": messages,
            "params": params,
            "real_text": real_text,
        },
    }))
}

fn guardrail_row(input: &Input, ops: &SurfaceOps) -> Option<Object> {
    let snapshot =
        GuardrailRequestSnapshot::capture(&input.snapshot_body, &input.snapshot_metadata);
    post_guardrail_kwargs(
        &input.kwargs,
        &input.metadata,
        ops.surface,
        snapshot.as_ref(),
    )
}

fn row(call_type: &str) -> Result<&'static SurfaceOps, Box<HelperError>> {
    match surface_ops(call_type) {
        Some(ops) => Ok(ops),
        None => Err(failure(
            "KeyError",
            format!("unknown call type: {call_type}"),
        )),
    }
}

fn new_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", error.to_string()))
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
