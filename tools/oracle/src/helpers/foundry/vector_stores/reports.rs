mod citation;
mod create;
mod credentials;

use betterlite_foundry::vector_stores::environment::{complete_url, endpoints_by_type};
use betterlite_foundry::vector_stores::search::{
    AzureSearchStoreRequest, SearchLogging, asearch_request, search_request,
};
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::Input;
use super::fixtures::{CallRecord, FixtureExecutor};
use super::scenarios::{
    AzureSearchEndpointInput, EmbeddedSearchScenario, EndpointScenario, SearchQueryFieldsInput,
    SearchScenario, StoreTypeEndpointsInput, VectorQueryWithEmbeddingInput,
};

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::StoreTypeEndpoints(input) => Ok(store_type_endpoints_report(input)),
        Input::AuthCredentials(input) => Ok(credentials::credentials_report(input)),
        Input::SearchQueryFields(input) => Ok(search_report(input)),
        Input::VectorQueryWithEmbedding(input) => embedded_report(input),
        Input::DocumentScoreCitation(input) => Ok(citation::citation_report(input)),
        Input::UnsupportedStoreCreate(input) => Ok(create::create_report(input)),
        Input::AzureSearchEndpoint(input) => Ok(endpoint_report(input)),
    }
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

fn search_report(input: &SearchQueryFieldsInput) -> Value {
    json!({
        "searches": input.searches.iter().map(search_outcome).collect::<Vec<Value>>(),
    })
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
    let extra_body = scenario.extra_body.as_ref().map_or(Value::Null, |body| {
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

fn endpoint_report(input: &AzureSearchEndpointInput) -> Value {
    json!({
        "scenarios": input.scenarios.iter().map(endpoint_outcome).collect::<Vec<Value>>(),
    })
}

fn endpoint_outcome(scenario: &EndpointScenario) -> Value {
    match complete_url(scenario.api_base.as_deref(), &scenario.litellm_params) {
        Ok(url) => json!({"ok": true, "url": url}),
        Err(_error) => json!({"ok": false, "status": Value::Null}),
    }
}
