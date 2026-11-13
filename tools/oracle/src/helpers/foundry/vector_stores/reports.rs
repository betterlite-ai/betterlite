use betterlite_foundry::vector_stores::create::{CreateStep, create_vector_store};
use betterlite_foundry::vector_stores::environment::{
    EnvironmentRequest, Headers, Params, auth_credentials, complete_url, endpoints_by_type,
    validate_environment,
};
use betterlite_foundry::vector_stores::error::AzureSearchError;
use betterlite_foundry::vector_stores::reply::{AzureSearchStoreReply, decode_azure_search_reply};
use betterlite_foundry::vector_stores::search::{
    AzureSearchStoreRequest, SearchLogging, asearch_request, search_request,
};
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreResultContent, VectorStoreSearchResponse, VectorStoreSearchResult,
};
use rust_decimal::prelude::ToPrimitive;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Number, Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::Input;
use super::fixtures::{CallRecord, FixtureExecutor, NoSources, empty_credentials};
use super::scenarios::{
    AuthCredentialsInput, AzureSearchEndpointInput, CitationScenario, DocumentScoreCitationInput,
    EmbeddedSearchScenario, EndpointScenario, HeaderScenario, SearchQueryFieldsInput,
    SearchScenario, StoreTypeEndpointsInput, UnsupportedStoreCreateInput,
    VectorQueryWithEmbeddingInput,
};

pub(super) fn run(input: &Input) -> HelperResult {
    Ok(match input {
        Input::StoreTypeEndpoints(input) => store_type_endpoints_report(input),
        Input::AuthCredentials(input) => credentials_report(input),
        Input::SearchQueryFields(input) => search_report(input),
        Input::VectorQueryWithEmbedding(input) => embedded_report(input)?,
        Input::DocumentScoreCitation(input) => citation_report(input),
        Input::UnsupportedStoreCreate(input) => create_report(input),
        Input::AzureSearchEndpoint(input) => endpoint_report(input),
    })
}

fn store_type_endpoints_report(_input: &StoreTypeEndpointsInput) -> Value {
    let endpoints = endpoints_by_type();
    json!({
        "endpoints": {
            "read": endpoints.read,
            "write": endpoints.write,
        },
    })
}

fn exposed(headers: &Headers) -> Value {
    let fields: serde_json::Map<String, Value> = headers
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                Value::String(value.expose_secret().to_owned()),
            )
        })
        .collect();
    Value::Object(fields)
}

fn credentials_report(input: &AuthCredentialsInput) -> Value {
    let credentials: Vec<Value> = input
        .credentials
        .iter()
        .map(|params| match auth_credentials(params) {
            Ok(credentials) => json!({"ok": true, "headers": exposed(&credentials.headers)}),
            Err(_error) => json!({"ok": false}),
        })
        .collect();
    let environments: Vec<Value> = input
        .environments
        .iter()
        .map(header_report)
        .collect();
    json!({"credentials": credentials, "environments": environments})
}

fn header_report(scenario: &HeaderScenario) -> Value {
    let credentials = empty_credentials();
    let sources = NoSources;
    let key = scenario
        .api_key
        .as_ref()
        .map(|key| SecretString::from(key.as_str()));
    let headers: Headers = scenario
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect();
    let request = EnvironmentRequest {
        headers,
        api_key: key.as_ref(),
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    };
    match validate_environment(request) {
        Ok(headers) => json!({"ok": true, "headers": exposed(&headers)}),
        Err(_error) => json!({"ok": false}),
    }
}

fn search_report(input: &SearchQueryFieldsInput) -> Value {
    let searches: Vec<Value> = input.searches.iter().map(search_outcome).collect();
    json!({"searches": searches})
}

fn search_outcome(scenario: &SearchScenario) -> Value {
    let embedding = scenario
        .embedding
        .clone()
        .unwrap_or_else(|| vec![0.25, 0.5]);
    let executor = FixtureExecutor::new(vec![embedding]);
    let mut logging = SearchLogging::default();
    let call = search_request(AzureSearchStoreRequest {
        vector_store_id: &scenario.vector_store_id,
        query: &scenario.query,
        optional_params: &scenario.optional_params,
        api_base: &scenario.api_base,
        litellm_params: &scenario.litellm_params,
        logging: &mut logging,
        embedding_executor: &executor,
    });
    let calls = calls_report(&executor.calls());
    let logging = logging_report(&logging);
    let extra_body = scenario
        .extra_body
        .as_ref()
        .map_or(Value::Null, |body| {
            serde_json::to_value(body).unwrap_or(Value::Null)
        });
    match call {
        Ok(call) => json!({
            "ok": true,
            "url": call.url,
            "body": call.body,
            "logging": logging,
            "calls": calls,
            "extra_body": extra_body,
        }),
        Err(_error) => json!({
            "ok": false,
            "status": Value::Null,
            "url": Value::Null,
            "body": Value::Null,
            "logging": logging,
            "calls": calls,
            "extra_body": extra_body,
        }),
    }
}

fn logging_report(logging: &SearchLogging) -> Value {
    let field = |name: &str| {
        logging
            .model_call_details
            .get(name)
            .map_or(Value::Null, |value| {
                serde_json::to_value(value).unwrap_or(Value::Null)
            })
    };
    json!({
        "input": field("input"),
        "embedding_model": field("embedding_model"),
        "top_k": field("top_k"),
    })
}

fn calls_report(calls: &[CallRecord]) -> Value {
    Value::Array(
        calls
            .iter()
            .map(|call| {
                json!({
                    "model": call.model,
                    "query": call.query,
                    "configuration": call.configuration,
                })
            })
            .collect(),
    )
}

fn embedded_report(input: &VectorQueryWithEmbeddingInput) -> HelperResult {
    let mut scenarios = Vec::with_capacity(input.scenarios.len());
    for scenario in &input.scenarios {
        scenarios.push(embedded_outcome(scenario)?);
    }
    Ok(json!({"scenarios": scenarios}))
}

fn embedded_outcome(scenario: &EmbeddedSearchScenario) -> Result<Value, Box<HelperError>> {
    let executor = FixtureExecutor::new(vec![scenario.embedding.clone()]);
    let mut logging = SearchLogging::default();
    let call = if scenario.asynchronous {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| runtime_error(error.to_string()))?;
        runtime.block_on(asearch_request(AzureSearchStoreRequest {
            vector_store_id: &scenario.vector_store_id,
            query: &scenario.query,
            optional_params: &scenario.optional_params,
            api_base: &scenario.api_base,
            litellm_params: &scenario.litellm_params,
            logging: &mut logging,
            embedding_executor: &executor,
        }))
    } else {
        search_request(AzureSearchStoreRequest {
            vector_store_id: &scenario.vector_store_id,
            query: &scenario.query,
            optional_params: &scenario.optional_params,
            api_base: &scenario.api_base,
            litellm_params: &scenario.litellm_params,
            logging: &mut logging,
            embedding_executor: &executor,
        })
    };
    let calls = calls_report(&executor.calls());
    match call {
        Ok(call) => Ok(json!({"ok": true, "url": call.url, "body": call.body, "calls": calls})),
        Err(_error) => Ok(json!({
            "ok": false,
            "status": Value::Null,
            "url": Value::Null,
            "body": Value::Null,
            "calls": calls,
        })),
    }
}

fn runtime_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "RuntimeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn citation_report(input: &DocumentScoreCitationInput) -> Value {
    let responses: Vec<Value> = input.responses.iter().map(citation_outcome).collect();
    json!({"responses": responses})
}

fn citation_outcome(scenario: &CitationScenario) -> Value {
    let mut logging = SearchLogging::default();
    if let Some(search) = &scenario.before_search {
        let executor = FixtureExecutor::new(vec![vec![0.25, 0.5]]);
        let call = search_request(AzureSearchStoreRequest {
            vector_store_id: &search.vector_store_id,
            query: &search.query,
            optional_params: &Params::new(),
            api_base: &search.api_base,
            litellm_params: &search.litellm_params,
            logging: &mut logging,
            embedding_executor: &executor,
        });
        if call.is_err() {
            return json!({"ok": false, "status": Value::Null});
        }
    }
    let reply = AzureSearchStoreReply {
        status_code: scenario.status,
        headers: &scenario.headers,
        body: scenario.body.as_bytes(),
        logging: &logging,
    };
    match decode_azure_search_reply(&reply) {
        Ok(page) => page_report(&page),
        Err(error) => json!({"ok": false, "status": error_status(&error)}),
    }
}

fn error_status(error: &AzureSearchError) -> Value {
    match error {
        AzureSearchError::ReplyJson { status_code, .. }
        | AzureSearchError::ReplyShape { status_code, .. } => Value::from(*status_code),
        _ => Value::Null,
    }
}

fn page_report(page: &VectorStoreSearchResponse) -> Value {
    let data = page.data.as_ref().map_or(Value::Null, |data| {
        Value::Array(data.iter().map(result_report).collect())
    });
    json!({
        "ok": true,
        "object": page.object,
        "search_query": page.search_query,
        "data": data,
    })
}

fn result_report(result: &VectorStoreSearchResult) -> Value {
    json!({
        "score": result.score.as_ref().map_or(Value::Null, score_value),
        "content": content_report(result.content.as_deref()),
        "file_id": result.file_id,
        "filename": result.filename,
        "attributes": result.attributes,
    })
}

fn content_report(content: Option<&[VectorStoreResultContent]>) -> Value {
    content.map_or(Value::Null, |items| {
        Value::Array(
            items
                .iter()
                .map(|item| json!({"text": item.text, "type": item.type_name}))
                .collect(),
        )
    })
}

// the Python response carries the score as a float, so a zero score goes out as 0.0 and not as 0
fn score_value(score: &Score) -> Value {
    score
        .0
        .to_f64()
        .and_then(Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

fn create_report(_input: &UnsupportedStoreCreateInput) -> Value {
    json!({
        "request": create_outcome(CreateStep::Request),
        "response": create_outcome(CreateStep::Response),
    })
}

fn create_outcome(step: CreateStep) -> Value {
    match create_vector_store(step) {
        Ok(_never) => json!({"ok": true}),
        Err(_error) => json!({"ok": false, "status": Value::Null}),
    }
}

fn endpoint_report(input: &AzureSearchEndpointInput) -> Value {
    let scenarios: Vec<Value> = input
        .scenarios
        .iter()
        .map(endpoint_outcome)
        .collect();
    json!({"scenarios": scenarios})
}

fn endpoint_outcome(scenario: &EndpointScenario) -> Value {
    match complete_url(scenario.api_base.as_deref(), &scenario.litellm_params) {
        Ok(url) => json!({"ok": true, "url": url}),
        Err(_error) => json!({"ok": false, "status": Value::Null}),
    }
}
