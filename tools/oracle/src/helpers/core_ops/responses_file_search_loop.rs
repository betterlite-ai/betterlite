use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_file_search_loop::ports::{
    FollowUpRequest, ResponsesFollowUp, SearchRequest, VectorStoreSearches,
};
use betterlite_core::api::responses_file_search_loop::records::{
    FileSearchDiagnostic, FollowUpCall, SearchAuthorization, UpstreamSearch,
};
use betterlite_core::api::responses_file_search_loop::{
    ResponsesFileSearchLoop, ResponsesFileSearchLoopInput,
};
use betterlite_core::api::responses_tool_namespaces::values::Fields;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::runtime::Builder;

type Failure = Box<HelperError>;

// the call id the emulation mints; the case ignores it against LiteLLM's uuid4
const GENERATED_ID: &str = "0123456789abcdef0123456789abcdef";
const NOW: i64 = 1_700_000_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    input: Json,
    model: String,
    tools: Option<Vec<Json>>,
    kwargs: Fields,
    provider: Vec<Json>,
    searches: Vec<ScriptedSearch>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ScriptedSearch {
    Result {
        vector_store_id: String,
        query: String,
        response: Json,
    },
    Error {
        vector_store_id: String,
        query: String,
        error: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let follow_up = ScriptedResponses::new(&input.provider)?;
        let searches = ScriptedSearches::new(&input.searches)?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        let runner = ResponsesFileSearchLoop {
            follow_up: &follow_up,
            searches: &searches,
            limits: ApiLimits::default(),
            ids: Arc::new(|| String::from(GENERATED_ID)),
            clock: Arc::new(|| Decimal::from(NOW)),
        };
        let case = ResponsesFileSearchLoopInput {
            input: input.input.clone(),
            model: input.model.clone(),
            tools: input.tools.clone(),
            kwargs: input.kwargs.clone(),
            authorization: SearchAuthorization::default(),
        };
        let result = runtime
            .block_on(runner.run(&case))
            .map_err(|error| failure("FileSearchLoopError", error.to_string()))?;
        Ok(json!({
            "response": serde_json::to_value(&result.response)?,
            "calls": result.calls.iter().map(call_value).collect::<Vec<Value>>(),
            "searches": result.searches.iter().map(search_value).collect::<Vec<Value>>(),
            "diagnostics": result.diagnostics.iter().map(line_value).collect::<Vec<Value>>(),
        }))
    }
}

fn call_value(call: &FollowUpCall) -> Value {
    json!({
        "input": &call.input,
        "model": &call.model,
        "tools": &call.tools,
        "kwargs": &call.kwargs,
    })
}

fn search_value(search: &UpstreamSearch) -> Value {
    json!({"vector_store_id": &search.vector_store_id, "query": &search.query})
}

fn line_value(line: &FileSearchDiagnostic) -> Value {
    json!({"level": line.level, "message": &line.message})
}

#[derive(Debug, Error)]
#[error("{0}")]
struct ScriptedError(String);

struct ScriptedResponses {
    responses: Mutex<VecDeque<ResponsesApiResponse>>,
    calls: Mutex<Vec<FollowUpCall>>,
}

impl ScriptedResponses {
    fn new(payloads: &[Json]) -> Result<Self, Failure> {
        let mut responses = VecDeque::new();
        for payload in payloads {
            let value = serde_json::to_value(payload)?;
            let response: ResponsesApiResponse = serde_json::from_value(value)
                .map_err(|error| failure("ValidationError", error.to_string()))?;
            responses.push_back(response);
        }
        Ok(Self {
            responses: Mutex::new(responses),
            calls: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl ResponsesFollowUp for ScriptedResponses {
    type Error = ScriptedError;

    async fn call(
        &self,
        request: FollowUpRequest<'_>,
    ) -> Result<ResponsesApiResponse, ScriptedError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FollowUpCall {
                input: request.input.clone(),
                model: String::from(request.model),
                tools: request.tools.map(<[Json]>::to_vec),
                kwargs: request.kwargs.clone(),
                internal: request.internal,
            });
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| ScriptedError(String::from("no scripted provider response")))
    }
}

struct ScriptedSearches {
    scripted: Vec<(String, String, Result<VectorStoreSearchResponse, String>)>,
    requests: Mutex<Vec<(String, String)>>,
}

impl ScriptedSearches {
    fn new(searches: &[ScriptedSearch]) -> Result<Self, Failure> {
        let mut scripted = Vec::with_capacity(searches.len());
        for search in searches {
            match search {
                ScriptedSearch::Result {
                    vector_store_id,
                    query,
                    response,
                } => {
                    let value = serde_json::to_value(response)?;
                    let parsed: VectorStoreSearchResponse = serde_json::from_value(value)
                        .map_err(|error| failure("ValidationError", error.to_string()))?;
                    scripted.push((vector_store_id.clone(), query.clone(), Ok(parsed)));
                }
                ScriptedSearch::Error {
                    vector_store_id,
                    query,
                    error,
                } => {
                    scripted.push((vector_store_id.clone(), query.clone(), Err(error.clone())));
                }
            }
        }
        Ok(Self {
            scripted,
            requests: Mutex::new(Vec::new()),
        })
    }

    fn scripted_result(
        &self,
        vector_store_id: &str,
        query: &str,
    ) -> Result<VectorStoreSearchResponse, ScriptedError> {
        for (store, name, found) in &self.scripted {
            if store == vector_store_id && name == query {
                return found.clone().map_err(ScriptedError);
            }
        }
        Err(ScriptedError(String::from(
            "no scripted search for this store and query",
        )))
    }
}

#[async_trait]
impl VectorStoreSearches for ScriptedSearches {
    type Error = ScriptedError;

    async fn asearch(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<VectorStoreSearchResponse, ScriptedError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                String::from(request.vector_store_id),
                String::from(request.query),
            ));
        self.scripted_result(request.vector_store_id, request.query)
    }
}

fn failure(class: &str, message: String) -> Failure {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
