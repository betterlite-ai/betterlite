use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::{Mode, RefuseDownloads};
use betterlite_openai::deepinfra::chat::{self, ChatTransform, MapRequest};
use betterlite_openai::deepinfra::rerank::response::{
    LoggingFailure, RerankLogging, ResponseRequest, get_error_class, transform_rerank_response,
};
use betterlite_openai::deepinfra::rerank::{
    EnvironmentRequest, Headers, MapRequest as RerankMapRequest, Params, RerankLimits,
    complete_url, map_cohere_rerank_params, transform_rerank_request, validate_environment,
};
use betterlite_openai::reasoning_models::DropParams;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::DeserializeOwned;
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

fn list(params: &Value) -> Result<&[Value], String> {
    params
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| String::from("params must be a list of cases"))
}

fn text<'value>(case: &'value Value, name: &str) -> Result<&'value str, String> {
    case[name]
        .as_str()
        .ok_or_else(|| format!("{name} must be a string"))
}

fn flag(case: &Value, name: &str) -> Result<bool, String> {
    case[name]
        .as_bool()
        .ok_or_else(|| format!("{name} must be a boolean"))
}

fn number(case: &Value, name: &str) -> Result<f64, String> {
    case[name]
        .as_f64()
        .ok_or_else(|| format!("{name} must be a number"))
}

fn object(value: &Value) -> Result<betterlite_prompts::arguments::json::Object, String> {
    from_value(value)
}

fn from_value<Decoded: DeserializeOwned>(value: &Value) -> Result<Decoded, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

fn chat_map(params: &Value) -> Result<Value, String> {
    let mut results = Vec::new();
    for case in list(params)? {
        let non_default = object(&case["params"])?;
        let mut optional = object(&case["optional_params"])?;
        let outcome = chat::map_openai_params(MapRequest {
            model: text(case, "model")?,
            non_default_params: &non_default,
            optional_params: &mut optional,
            drop_params: DropParams {
                global: flag(case, "global_drop_params")?,
                request: flag(case, "drop_params")?,
            },
            minimum_temperature: number(case, "minimum_temperature")?,
            supports_reasoning: flag(case, "supports_reasoning")?,
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
    for case in list(params)? {
        let messages: Vec<ChatMessage> = from_value(&case["messages"])?;
        let optional = object(&case["optional_params"])?;
        let environment: BTreeMap<String, String> = BTreeMap::new();
        let body = chat::transform_request(
            &ChatTransform {
                model: text(case, "model")?,
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
    for case in list(params)? {
        let explicit = case["api_key"].as_str().map(SecretString::from);
        let fallback = case["environment_key"].as_str().map(SecretString::from);
        let headers: Headers = from_value(&case["headers"])?;
        let merged = validate_environment(&EnvironmentRequest {
            headers: &headers,
            api_key: explicit.as_ref(),
            environment_key: fallback.as_ref(),
        })
        .map_err(|error| error.to_string())?;
        let url = complete_url(case["api_base"].as_str(), text(case, "model")?)
            .map_err(|error| error.to_string())?;
        let non_default: Params = from_value(&case["non_default"])?;
        let documents: Vec<RerankDocument> = from_value(&case["documents"])?;
        let mapped = map_cohere_rerank_params(&RerankMapRequest {
            non_default_params: &non_default,
            query: text(case, "query")?,
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
    for case in list(params)? {
        let mut recording = Recording {
            calls: Vec::new(),
            fail: flag(case, "fail_logging")?,
        };
        let projected = transform_rerank_response(
            RerankResponse::default(),
            ResponseRequest {
                model: text(case, "model")?,
                body: text(case, "body")?.as_bytes(),
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
    for case in list(params)? {
        let headers: BTreeMap<String, Json> = from_value(&case["headers"])?;
        let raw = case["status_code"]
            .as_u64()
            .ok_or_else(|| String::from("status_code must be an integer"))?;
        let status =
            u16::try_from(raw).map_err(|_source| String::from("status_code is out of range"))?;
        let raised = get_error_class(
            text(case, "message")?,
            status,
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
