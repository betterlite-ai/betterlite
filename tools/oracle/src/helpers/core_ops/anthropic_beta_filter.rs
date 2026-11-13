use betterlite_core::api::anthropic_beta_config::matrix::{BetaMatrix, BetaNode};
use betterlite_core::api::anthropic_beta_filter::Headers;
use betterlite_core::api::anthropic_beta_filter::matrix_filter::{
    DroppedBeta, filter_beta_tokens, filter_request_betas,
};
use betterlite_core::api::anthropic_beta_filter::native_injection::{
    NativeRequest, inject_native_betas,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    matrix: BetaNode,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    MatrixFilter {
        provider: String,
        tokens: Option<Vec<String>>,
        headers: Option<IndexMap<String, Option<String>>>,
        body: Option<IndexMap<String, Json>>,
    },
    NativeInjection {
        custom_llm_provider: String,
        headers: IndexMap<String, String>,
        optional_params: IndexMap<String, Json>,
        messages: Vec<Json>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let BetaNode::Map(entries) = &input.matrix else {
            return Err(failure(
                "the beta headers matrix is not an object".to_owned(),
            ));
        };
        let matrix = BetaMatrix(entries.clone());
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            let answer = match call {
                Call::MatrixFilter {
                    provider,
                    tokens,
                    headers,
                    body,
                } => matrix_filter_answer(
                    &matrix,
                    provider,
                    tokens.as_deref(),
                    headers.as_ref(),
                    body.as_ref(),
                )?,
                Call::NativeInjection {
                    custom_llm_provider,
                    headers,
                    optional_params,
                    messages,
                } => native_injection_answer(
                    custom_llm_provider,
                    headers,
                    optional_params,
                    messages,
                )?,
            };
            answers.push(answer);
        }
        Ok(json!({ "calls": answers }))
    }
}

fn matrix_filter_answer(
    matrix: &BetaMatrix,
    provider: &str,
    tokens: Option<&[String]>,
    headers: Option<&IndexMap<String, Option<String>>>,
    body: Option<&IndexMap<String, Json>>,
) -> Result<Value, Box<HelperError>> {
    let mut dropped: Vec<DroppedBeta> = Vec::new();
    let filtered = match tokens {
        None => None,
        Some(given) => {
            let answer = filter_beta_tokens(matrix, provider, given)
                .map_err(|error| failure(error.to_string()))?;
            dropped.extend(answer.dropped);
            Some(answer.tokens)
        }
    };
    let answer = request_answer(matrix, provider, headers, body, &mut dropped)?;
    Ok(json!({
        "headers": answer.headers,
        "body": answer.body,
        "tokens": filtered,
        "drops": dropped,
    }))
}

struct RequestAnswer {
    headers: Option<Map<String, Value>>,
    body: Option<IndexMap<String, Json>>,
}

fn request_answer(
    matrix: &BetaMatrix,
    provider: &str,
    headers: Option<&IndexMap<String, Option<String>>>,
    body: Option<&IndexMap<String, Json>>,
    dropped: &mut Vec<DroppedBeta>,
) -> Result<RequestAnswer, Box<HelperError>> {
    if headers.is_none() && body.is_none() {
        return Ok(RequestAnswer {
            headers: None,
            body: None,
        });
    }
    let original = headers.cloned().unwrap_or_default();
    let mut request_headers: Headers = original
        .iter()
        .filter_map(|(name, value)| value.as_ref().map(|text| (name.clone(), text.clone())))
        .collect();
    let mut request_body = body.cloned().unwrap_or_default();
    let request_drops =
        filter_request_betas(&mut request_headers, &mut request_body, matrix, provider)
            .map_err(|error| failure(error.to_string()))?;
    dropped.extend(request_drops);
    Ok(RequestAnswer {
        headers: Some(rebuild_headers(&original, &request_headers)),
        body: Some(request_body),
    })
}

fn rebuild_headers(
    original: &IndexMap<String, Option<String>>,
    updated: &Headers,
) -> Map<String, Value> {
    let mut answer = Map::new();
    for (name, value) in original {
        if value.is_none() {
            answer.insert(name.clone(), Value::Null);
            continue;
        }
        if let Some(text) = updated.get(name) {
            answer.insert(name.clone(), Value::String(text.clone()));
        }
    }
    answer
}

fn native_injection_answer(
    provider: &str,
    headers: &IndexMap<String, String>,
    optional_params: &IndexMap<String, Json>,
    messages: &[Json],
) -> Result<Value, Box<HelperError>> {
    let merged = inject_native_betas(
        headers.clone(),
        &NativeRequest {
            provider,
            optional_params,
            messages,
        },
    )
    .map_err(|error| failure(error.to_string()))?;
    Ok(json!({
        "headers": merged,
        "body": Value::Null,
        "tokens": Value::Null,
        "drops": [],
    }))
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BetaFilterError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
