use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::{Mode, RefuseDownloads};
use betterlite_openai::deepinfra::chat::{self, ChatTransform, MapRequest};
use betterlite_openai::deepinfra::rerank::error::get_error_class;
use betterlite_openai::deepinfra::rerank::response::{
    LoggingFailure, RerankLogging, ResponseRequest, transform_rerank_response,
};
use betterlite_openai::deepinfra::rerank::{
    EnvironmentRequest, Headers, MapRequest as RerankMapRequest, Params, RerankLimits,
    complete_url, map_cohere_rerank_params, transform_rerank_request, validate_environment,
};
use betterlite_openai::reasoning_models::DropParams;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

mod tests;

const LIMIT: usize = 0x0010_0000;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    ChatMap { params: Value },
    ToolContent { params: Value },
    RerankRequest { params: Value },
    RerankResponse { params: Value },
    ErrorExtraction { params: Value },
}

#[derive(Debug, Default, Deserialize)]
struct Case {
    #[serde(default)]
    model: String,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    optional_params: Value,
    #[serde(default)]
    minimum_temperature: f64,
    #[serde(default)]
    messages: Value,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    headers: Value,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    environment_key: Option<String>,
    #[serde(default)]
    query: String,
    #[serde(default)]
    documents: Vec<Value>,
    #[serde(default)]
    non_default: Value,
    #[serde(default)]
    body: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    status_code: u16,
    #[serde(flatten)]
    request_flags: RequestFlags,
    #[serde(flatten)]
    model_flags: ModelFlags,
}

#[derive(Debug, Default, Deserialize)]
struct RequestFlags {
    #[serde(default)]
    drop_params: bool,
    #[serde(default)]
    fail_logging: bool,
}

#[derive(Debug, Default, Deserialize)]
struct ModelFlags {
    #[serde(default)]
    global_drop_params: bool,
    #[serde(default)]
    supports_reasoning: bool,
}

struct Recording {
    calls: Vec<String>,
    fail: bool,
}

impl RerankLogging for Recording {
    fn post_call(&mut self, original_response: &str) -> Result<(), LoggingFailure> {
        if self.fail {
            return Err(LoggingFailure);
        }
        self.calls.push(original_response.to_owned());
        Ok(())
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::ChatMap { params } => chat_map(params),
            Input::ToolContent { params } => tool_content(params),
            Input::RerankRequest { params } => rerank_request(params),
            Input::RerankResponse { params } => rerank_response(params),
            Input::ErrorExtraction { params } => error_extraction(params),
        };
        output.map_err(|message| {
            Box::new(crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            })
        })
    }
}

fn cases(params: &Value) -> Result<Vec<Case>, String> {
    serde_json::from_value(params.clone()).map_err(|error| error.to_string())
}

fn object(value: &Value) -> Result<betterlite_prompts::arguments::json::Object, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

fn chat_map(params: &Value) -> Result<Value, String> {
    let mut results = Vec::new();
    for case in cases(params)? {
        let non_default = object(&case.params)?;
        let mut optional = object(&case.optional_params)?;
        let outcome = chat::map_openai_params(MapRequest {
            model: &case.model,
            non_default_params: &non_default,
            optional_params: &mut optional,
            drop_params: DropParams {
                global: case.model_flags.global_drop_params,
                request: case.request_flags.drop_params,
            },
            minimum_temperature: case.minimum_temperature,
            supports_reasoning: case.model_flags.supports_reasoning,
        });
        results.push(match outcome {
            Ok(()) => serde_json::json!({ "ok": true, "mapped": optional }),
            Err(error) => serde_json::json!({ "ok": false, "status": error.status_code() }),
        });
    }
    Ok(serde_json::json!({ "results": results }))
}

fn tool_content(params: &Value) -> Result<Value, String> {
    let mut results = Vec::new();
    for case in cases(params)? {
        let messages: Vec<ChatMessage> =
            serde_json::from_value(case.messages).map_err(|error| error.to_string())?;
        let optional = object(&case.optional_params)?;
        let environment: BTreeMap<String, String> = BTreeMap::new();
        let body = chat::transform_request(
            &ChatTransform {
                model: &case.model,
                messages: &messages,
                optional_params: &optional,
                environment: &environment,
                downloads: &RefuseDownloads,
            },
            Mode::Sync,
        )
        .map_err(|error| error.to_string())?;
        results
            .push(serde_json::to_value(body.get("messages")).map_err(|error| error.to_string())?);
    }
    Ok(serde_json::json!({ "results": results }))
}

fn rerank_request(params: &Value) -> Result<Value, String> {
    let mut results = Vec::new();
    for case in cases(params)? {
        let explicit = case.api_key.as_ref().map(SecretString::from);
        let environment = case.environment_key.as_ref().map(SecretString::from);
        let headers: Headers =
            serde_json::from_value(case.headers).map_err(|error| error.to_string())?;
        let merged = validate_environment(&EnvironmentRequest {
            headers: &headers,
            api_key: explicit.as_ref(),
            environment_key: environment.as_ref(),
        })
        .map_err(|error| error.to_string())?;
        let url = complete_url(case.api_base.as_deref(), &case.model)
            .map_err(|error| error.to_string())?;
        let non_default: Params =
            serde_json::from_value(case.non_default).map_err(|error| error.to_string())?;
        let documents: Vec<RerankDocument> = serde_json::from_value(Value::Array(case.documents))
            .map_err(|error| error.to_string())?;
        let mapped = map_cohere_rerank_params(&RerankMapRequest {
            non_default_params: &non_default,
            query: &case.query,
            documents: &documents,
        });
        let request = transform_rerank_request(Some(&mapped));
        results.push(serde_json::json!({
            "url": url,
            "headers": merged,
            "mapped": mapped,
            "request": request,
        }));
    }
    Ok(serde_json::json!({ "results": results }))
}

fn rerank_response(params: &Value) -> Result<Value, String> {
    let mut results = Vec::new();
    for case in cases(params)? {
        let mut recording = Recording {
            calls: Vec::new(),
            fail: case.request_flags.fail_logging,
        };
        let projected = transform_rerank_response(
            RerankResponse::default(),
            ResponseRequest {
                model: &case.model,
                body: case.body.as_bytes(),
                logging: &mut recording,
                generated: Uuid::from_u128(1),
                limits: RerankLimits {
                    max_body_bytes: LIMIT,
                },
            },
        );
        results.push(serde_json::json!({
            "response": serde_json::to_value(&projected).map_err(|error| error.to_string())?,
            "calls": recording.calls,
        }));
    }
    Ok(serde_json::json!({ "results": results }))
}

fn error_extraction(params: &Value) -> Result<Value, String> {
    let mut results = Vec::new();
    for case in cases(params)? {
        let headers: BTreeMap<String, Json> =
            serde_json::from_value(case.headers).map_err(|error| error.to_string())?;
        let raised = get_error_class(
            &case.message,
            case.status_code,
            headers,
            RerankLimits {
                max_body_bytes: LIMIT,
            },
        );
        results.push(serde_json::json!({
            "status_code": raised.status_code,
            "message": raised.message,
            "headers": raised.headers,
        }));
    }
    Ok(serde_json::json!({ "results": results }))
}
