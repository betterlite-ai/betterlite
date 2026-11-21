use std::convert::Infallible;
use std::slice::from_ref;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResult;
use indexmap::IndexMap;
use tracing::warn;

use crate::api::api_boundaries::ApiLimits;
use crate::api::chat_response_stream_tools::fields::string_value;
use crate::api::responses_file_search_tools::results::{
    NO_DEFAULT, field, format_results_as_tool_output,
};
use crate::api::responses_file_search_tools::tools::{
    extract_tool_call_fields, resolve_queries_from_args,
};
use crate::api::responses_tool_namespaces::values::Fields;

use super::decoder::tool_call_arguments;
use super::error::FileSearchLoopError;
use super::ports::{SearchRequest, VectorStoreSearches};
use super::records::{
    DiagnosticMessage, ExecutedToolCalls, FILE_SEARCH_LOGGER, FileSearchDiagnostic,
    SearchAuthorization, UpstreamSearch, WARNING_LEVEL,
};

const CALL_ID_FIELD: &str = "call_id";
const FUNCTION_CALL_OUTPUT_TYPE: &str = "function_call_output";
const OUTPUT_FIELD: &str = "output";
const VECTOR_STORE_ID_FIELD: &str = "vector_store_id";

// one _execute_file_search_tool_calls call's searches and warnings
#[derive(Debug, Default)]
pub struct SearchRound {
    pub searches: Vec<UpstreamSearch>,
    pub diagnostics: Vec<FileSearchDiagnostic>,
}

impl SearchRound {
    fn record(&mut self, vector_store_id: &str, query: &str, result_count: usize, failed: bool) {
        self.searches.push(UpstreamSearch {
            vector_store_id: String::from(vector_store_id),
            query: String::from(query),
            result_count,
            failed,
        });
    }
}

pub struct ToolCallRequest<'round> {
    pub tool_calls: &'round [Fields],
    pub vector_store_ids: &'round [String],
    pub input: &'round Json,
    pub fallback_call_id: &'round str,
    pub limits: &'round ApiLimits,
    pub authorization: &'round SearchAuthorization,
}

// _run_vector_searches (L140-L178): one asearch per query per store, query-major, and a failed
// search is a warning rather than an error, exactly as the bare except there treats it
pub async fn run_vector_searches<Searches>(
    searches: &Searches,
    vector_store_ids: &[String],
    queries: &[String],
    round: &mut SearchRound,
    authorization: &SearchAuthorization,
) -> Vec<VectorStoreSearchResult>
where
    Searches: VectorStoreSearches + ?Sized,
{
    let mut collected: Vec<VectorStoreSearchResult> = Vec::new();
    for query in queries {
        for vector_store_id in vector_store_ids {
            let request = SearchRequest {
                vector_store_id,
                query,
                authorization,
            };
            match searches.asearch(request).await {
                Ok(response) => {
                    let found = response.data.unwrap_or_default();
                    round.record(vector_store_id, query, found.len(), false);
                    collected.extend(found);
                }
                Err(error) => {
                    let failure = error.to_string();
                    let message = DiagnosticMessage::SearchFailed {
                        query,
                        vector_store_id,
                        error: &failure,
                    }
                    .to_string();
                    warn!(target: FILE_SEARCH_LOGGER, "{message}");
                    round.record(vector_store_id, query, 0, true);
                    round.diagnostics.push(FileSearchDiagnostic {
                        level: WARNING_LEVEL,
                        message,
                    });
                }
            }
        }
    }
    collected
}

// _execute_file_search_tool_calls (L440-L486)
pub async fn execute_file_search_tool_calls<Searches>(
    searches: &Searches,
    request: &ToolCallRequest<'_>,
) -> Result<ExecutedToolCalls, FileSearchLoopError<Infallible>>
where
    Searches: VectorStoreSearches + ?Sized,
{
    let mut round = SearchRound::default();
    let mut all_queries: Vec<String> = Vec::new();
    let mut all_results: Vec<VectorStoreSearchResult> = Vec::new();
    let mut outputs: Vec<Json> = Vec::new();
    for tool_call in request.tool_calls {
        let (call_id, raw_arguments) =
            extract_tool_call_fields(tool_call, request.fallback_call_id)?;
        let arguments = tool_call_arguments(&raw_arguments, request.limits)?;
        let queries = resolve_queries_from_args(&arguments, request.input)?;
        store_outside(&mut round, request.vector_store_ids, &arguments);
        let stores = vector_stores_for_call(&arguments, request.vector_store_ids);
        let results = run_vector_searches(
            searches,
            stores,
            &queries,
            &mut round,
            request.authorization,
        )
        .await;
        all_queries.extend(queries);
        outputs.push(function_call_output(&call_id, &results));
        all_results.extend(results);
    }
    Ok(ExecutedToolCalls {
        call_id: String::from(request.fallback_call_id),
        queries: all_queries,
        results: all_results,
        outputs,
        searches: round.searches,
        diagnostics: round.diagnostics,
    })
}

// the model's own vector_store_id wins only when the request lists it (L469)
fn vector_stores_for_call<'all>(call: &Fields, all: &'all [String]) -> &'all [String] {
    let taken = field(call, VECTOR_STORE_ID_FIELD, &NO_DEFAULT);
    let wanted = string_value(&taken);
    match all.iter().find(|known| **known == wanted) {
        Some(known) => from_ref(known),
        None => all,
    }
}

// the warning when the model names a vector store the request did not (L461-L468)
fn store_outside(round: &mut SearchRound, all: &[String], call: &Fields) {
    let taken = field(call, VECTOR_STORE_ID_FIELD, &NO_DEFAULT);
    let wanted = string_value(&taken);
    if taken == Json::Null || all.iter().any(|known| *known == wanted) {
        return;
    }
    let message = DiagnosticMessage::StoreOutside {
        value: &taken,
        known: all,
    }
    .to_string();
    warn!(target: FILE_SEARCH_LOGGER, "{message}");
    round.diagnostics.push(FileSearchDiagnostic {
        level: WARNING_LEVEL,
        message,
    });
}

// {"type": "function_call_output", "call_id": ..., "output": ...}, in that key order (L478-L484)
fn function_call_output(call_id: &str, results: &[VectorStoreSearchResult]) -> Json {
    Json::Object(IndexMap::from([
        (
            String::from("type"),
            Json::Text(String::from(FUNCTION_CALL_OUTPUT_TYPE)),
        ),
        (
            String::from(CALL_ID_FIELD),
            Json::Text(String::from(call_id)),
        ),
        (
            String::from(OUTPUT_FIELD),
            Json::Text(format_results_as_tool_output(results)),
        ),
    ]))
}
