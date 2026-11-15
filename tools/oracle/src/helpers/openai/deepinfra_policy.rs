use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::{Mode, RefuseDownloads};
use betterlite_openai::deepinfra::chat::{self, ChatTransform, MapRequest};
use betterlite_openai::deepinfra::rerank::response::error::get_error_class;
use betterlite_openai::deepinfra::rerank::response::{
    LoggingFailure, RerankLogging, ResponseRequest, transform_rerank_response,
};
use betterlite_openai::deepinfra::rerank::{
    EnvironmentRequest, HeaderMap, Headers, MapRequest as RerankMapRequest, Params, RerankLimits,
    complete_url, map_cohere_rerank_params, transform_rerank_request, validate_environment,
};
use betterlite_openai::reasoning_models::DropParams;
use betterlite_prompts::arguments::json::{Json as MessageJson, Object};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

mod tests;

const LIMIT: usize = 0x0010_0000;
const LIMITS: RerankLimits = RerankLimits {
    max_body_bytes: LIMIT,
};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    ChatMap { params: Vec<ChatMapCase> },
    ToolContent { params: Vec<ToolContentCase> },
    RerankRequest { params: Vec<RerankRequestCase> },
    RerankResponse { params: Vec<RerankResponseCase> },
    ErrorExtraction { params: Vec<ErrorExtractionCase> },
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatMapCase {
    model: String,
    params: Object,
    optional_params: Object,
    #[serde(flatten)]
    drops: DropFlags,
    supports_reasoning: bool,
    minimum_temperature: f64,
}

#[derive(Debug, Deserialize)]
struct DropFlags {
    drop_params: bool,
    global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolContentCase {
    model: String,
    messages: Vec<ChatMessage>,
    optional_params: Object,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RerankRequestCase {
    api_base: Option<String>,
    model: String,
    headers: Headers,
    api_key: Option<SecretString>,
    environment_key: Option<SecretString>,
    query: String,
    documents: Vec<RerankDocument>,
    non_default: Params,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RerankResponseCase {
    model: String,
    body: String,
    fail_logging: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorExtractionCase {
    message: String,
    status_code: u16,
    headers: HeaderMap,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ChatMapReply {
    Mapped { ok: bool, mapped: Object },
    Rejected { ok: bool, status: u16 },
}

#[derive(Debug, Serialize)]
struct RerankRequestReply {
    url: String,
    headers: Headers,
    mapped: Params,
    request: Params,
}

#[derive(Debug, Serialize)]
struct RerankResponseReply {
    response: RerankResponse,
    hidden_params: BTreeMap<String, BTreeMap<String, Json>>,
    calls: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ErrorExtractionReply {
    status_code: u16,
    message: String,
    headers: HeaderMap,
}

#[derive(Debug, Serialize)]
struct CaseResults<Reply> {
    results: Vec<Reply>,
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
            Input::ChatMap { params } => Ok(case_values(params, chat_case)),
            Input::ToolContent { params } => case_results(params, tool_content_case),
            Input::RerankRequest { params } => case_results(params, rerank_request_case),
            Input::RerankResponse { params } => Ok(case_values(params, rerank_response_case)),
            Input::ErrorExtraction { params } => Ok(case_values(params, error_extraction_case)),
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

fn case_values<Case, Reply>(cases: &[Case], convert: fn(&Case) -> Reply) -> Value
where
    Reply: Serialize,
{
    let mut values = Vec::new();
    for case in cases {
        values.push(convert(case));
    }
    serde_json::json!({ "results": values })
}

fn case_results<Case, Reply>(
    cases: &[Case],
    convert: fn(&Case) -> Result<Reply, String>,
) -> Result<Value, String>
where
    Reply: Serialize,
{
    let results = cases
        .iter()
        .map(convert)
        .collect::<Result<Vec<Reply>, String>>()?;
    serde_json::to_value(CaseResults { results }).map_err(|error| error.to_string())
}

fn chat_case(case: &ChatMapCase) -> ChatMapReply {
    let mut optional = case.optional_params.clone();
    let outcome = chat::map_openai_params(MapRequest {
        model: &case.model,
        non_default_params: &case.params,
        optional_params: &mut optional,
        drop_params: DropParams {
            global: case.drops.global_drop_params,
            request: case.drops.drop_params,
        },
        minimum_temperature: case.minimum_temperature,
        supports_reasoning: case.supports_reasoning,
    });
    match outcome {
        Ok(()) => ChatMapReply::Mapped {
            ok: true,
            mapped: optional,
        },
        Err(error) => ChatMapReply::Rejected {
            ok: false,
            status: error.status_code(),
        },
    }
}

fn tool_content_case(case: &ToolContentCase) -> Result<MessageJson, String> {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let body = chat::transform_request(
        &ChatTransform {
            model: &case.model,
            messages: &case.messages,
            optional_params: &case.optional_params,
            environment: &environment,
            downloads: &RefuseDownloads,
        },
        Mode::Sync,
    )
    .map_err(|error| error.to_string())?;
    Ok(body.get("messages").cloned().unwrap_or(MessageJson::Null))
}

fn rerank_request_case(case: &RerankRequestCase) -> Result<RerankRequestReply, String> {
    let mapped = map_cohere_rerank_params(&RerankMapRequest {
        non_default_params: &case.non_default,
        query: &case.query,
        documents: &case.documents,
    });
    Ok(RerankRequestReply {
        url: complete_url(case.api_base.as_deref(), &case.model)
            .map_err(|error| error.to_string())?,
        headers: validate_environment(&EnvironmentRequest {
            headers: &case.headers,
            api_key: case.api_key.as_ref(),
            environment_key: case.environment_key.as_ref(),
        })
        .map_err(|error| error.to_string())?,
        request: transform_rerank_request(Some(&mapped)),
        mapped,
    })
}

fn rerank_response_case(case: &RerankResponseCase) -> RerankResponseReply {
    let mut recording = Recording {
        calls: Vec::new(),
        fail: case.fail_logging,
    };
    let projected = transform_rerank_response(
        RerankResponse::default(),
        ResponseRequest {
            model: &case.model,
            body: case.body.as_bytes(),
            logging: &mut recording,
            generated: Uuid::from_u128(1),
            limits: LIMITS,
        },
    );
    RerankResponseReply {
        // the Python record reads pydantic's private attribute map, which nests the returned map under its name
        hidden_params: BTreeMap::from([(
            String::from("_hidden_params"),
            projected.hidden_params.clone(),
        )]),
        response: projected,
        calls: recording.calls,
    }
}

fn error_extraction_case(case: &ErrorExtractionCase) -> ErrorExtractionReply {
    let raised = get_error_class(
        &case.message,
        case.status_code,
        case.headers.clone(),
        LIMITS,
    );
    ErrorExtractionReply {
        status_code: raised.status_code,
        message: raised.message,
        headers: raised.headers,
    }
}
