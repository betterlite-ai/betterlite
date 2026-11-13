use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::{Mode, RefuseDownloads};
use betterlite_openai::deepinfra::chat::{self, ChatTransform, MapRequest};
use betterlite_openai::deepinfra::rerank::{
    self, EnvironmentRequest, Headers, LoggingFailure, MapRequest as RerankMapRequest, Params,
    RerankLimits, RerankLogging, ResponseRequest,
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

#[derive(Debug, Deserialize)]
struct Cases<Case> {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct ChatMapCase {
    model: String,
    params: Value,
    optional_params: Value,
    drop_params: bool,
    global_drop_params: bool,
    supports_reasoning: bool,
    minimum_temperature: f64,
}

#[derive(Debug, Deserialize)]
struct ToolCase {
    model: String,
    messages: Value,
    optional_params: Value,
}

#[derive(Debug, Deserialize)]
struct RerankRequestCase {
    api_base: Option<String>,
    model: String,
    headers: Headers,
    api_key: Option<String>,
    environment_key: Option<String>,
    query: String,
    documents: Vec<Value>,
    non_default: Value,
}

#[derive(Debug, Deserialize)]
struct RerankResponseCase {
    model: String,
    body: String,
    fail_logging: bool,
}

#[derive(Debug, Deserialize)]
struct ErrorCase {
    message: String,
    status_code: u16,
    headers: BTreeMap<String, Json>,
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

fn prompt_object(value: &Value) -> Result<betterlite_prompts::arguments::json::Object, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

fn chat_map(params: &Value) -> Result<Value, String> {
    let parsed: Cases<ChatMapCase> =
        serde_json::from_value(params.clone()).map_err(|error| error.to_string())?;
    let mut results = Vec::with_capacity(parsed.cases.len());
    for case in parsed.cases {
        results.push(chat_map_case(&case)?);
    }
    Ok(serde_json::json!({ "results": results }))
}

fn chat_map_case(case: &ChatMapCase) -> Result<Value, String> {
    let non_default = prompt_object(&case.params)?;
    let mut optional = prompt_object(&case.optional_params)?;
    let outcome = chat::map_openai_params(MapRequest {
        model: &case.model,
        non_default_params: &non_default,
        optional_params: &mut optional,
        drop_params: DropParams {
            global: case.global_drop_params,
            request: case.drop_params,
        },
        minimum_temperature: case.minimum_temperature,
        supports_reasoning: case.supports_reasoning,
    });
    Ok(match outcome {
        Ok(()) => serde_json::json!({ "ok": true, "mapped": optional }),
        Err(error) => serde_json::json!({ "ok": false, "status": error.status_code() }),
    })
}

fn tool_content(params: &Value) -> Result<Value, String> {
    let parsed: Cases<ToolCase> =
        serde_json::from_value(params.clone()).map_err(|error| error.to_string())?;
    let mut results = Vec::with_capacity(parsed.cases.len());
    for case in parsed.cases {
        results.push(tool_content_case(&case)?);
    }
    Ok(serde_json::json!({ "results": results }))
}

fn tool_content_case(case: &ToolCase) -> Result<Value, String> {
    let messages: Vec<ChatMessage> =
        serde_json::from_value(case.messages.clone()).map_err(|error| error.to_string())?;
    let optional = prompt_object(&case.optional_params)?;
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
    let messages = body
        .get("messages")
        .ok_or_else(|| String::from("the transformed body has no messages"))?;
    serde_json::to_value(messages).map_err(|error| error.to_string())
}

fn rerank_request(params: &Value) -> Result<Value, String> {
    let parsed: Cases<RerankRequestCase> =
        serde_json::from_value(params.clone()).map_err(|error| error.to_string())?;
    let mut results = Vec::with_capacity(parsed.cases.len());
    for case in parsed.cases {
        results.push(rerank_request_case(&case)?);
    }
    Ok(serde_json::json!({ "results": results }))
}

fn rerank_request_case(case: &RerankRequestCase) -> Result<Value, String> {
    let explicit = case.api_key.as_ref().map(SecretString::from);
    let environment = case.environment_key.as_ref().map(SecretString::from);
    let headers = rerank::validate_environment(&EnvironmentRequest {
        headers: &case.headers,
        api_key: explicit.as_ref(),
        environment_key: environment.as_ref(),
    })
    .map_err(|error| error.to_string())?;
    let url = rerank::complete_url(case.api_base.as_deref(), &case.model)
        .map_err(|error| error.to_string())?;
    let non_default: Params =
        serde_json::from_value(case.non_default.clone()).map_err(|error| error.to_string())?;
    let documents: Vec<RerankDocument> =
        serde_json::from_value(Value::Array(case.documents.clone()))
            .map_err(|error| error.to_string())?;
    let mapped = rerank::map_cohere_rerank_params(&RerankMapRequest {
        non_default_params: &non_default,
        query: &case.query,
        documents: &documents,
    });
    let request = rerank::transform_rerank_request(Some(&mapped));
    Ok(serde_json::json!({
        "url": url,
        "headers": headers,
        "mapped": mapped,
        "request": request,
    }))
}

fn rerank_response(params: &Value) -> Result<Value, String> {
    let parsed: Cases<RerankResponseCase> =
        serde_json::from_value(params.clone()).map_err(|error| error.to_string())?;
    let mut results = Vec::with_capacity(parsed.cases.len());
    for case in parsed.cases {
        results.push(rerank_response_case(&case)?);
    }
    Ok(serde_json::json!({ "results": results }))
}

fn rerank_response_case(case: &RerankResponseCase) -> Result<Value, String> {
    let mut recording = Recording {
        calls: Vec::new(),
        fail: case.fail_logging,
    };
    let projected = rerank::transform_rerank_response(
        betterlite_openai::api::rerank_contracts::records::RerankResponse::default(),
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
    let response = serde_json::to_value(&projected).map_err(|error| error.to_string())?;
    Ok(serde_json::json!({ "response": response, "calls": recording.calls }))
}

fn error_extraction(params: &Value) -> Result<Value, String> {
    let parsed: Cases<ErrorCase> =
        serde_json::from_value(params.clone()).map_err(|error| error.to_string())?;
    let mut results = Vec::with_capacity(parsed.cases.len());
    for case in parsed.cases {
        let raised = rerank::get_error_class(
            &case.message,
            case.status_code,
            case.headers,
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
