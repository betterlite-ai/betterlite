use betterlite_core::api::vector_query_embedding::query::{
    SelectedExecutor, query_embedding_executor,
};
use betterlite_models::decisions::Json;
use betterlite_provider::api::vector_query_embedding::query::{
    QueryEmbeddingConfig as _, QuerySearchRequest, embedding_configuration,
    query_embedding_configuration, query_embedding_model, query_text,
};
use betterlite_provider::api::vector_store_contracts::config::{
    Params, SearchQuery, SearchRequest, VectorStoreConfig as _, VectorStoreEmbeddingExecutor,
    VectorStoreLogging, VectorStoreResponse,
};
use fixtures::{DirectFixture, ExplicitExecutor, QueryFixture, RouterFixture, direct_executor};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::helpers::HelperError;

mod fixtures;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    QueryPreparation {
        queries: Vec<SearchQuery>,
        litellm_params: Vec<PreparationCase>,
    },
    ExecutorPrecedence {
        cases: Vec<PrecedenceCase>,
        forward: ForwardInput,
    },
    DirectStoreDefaults {
        api_bases: Vec<Option<String>>,
        #[serde(default)]
        litellm_params: Params,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparationCase {
    #[serde(default)]
    model: Option<Json>,
    #[serde(default)]
    config: Option<Slot>,
}

// a Python caller can build a dict with keys that are not strings, which JSON objects cannot carry
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Slot {
    Entries { entries: Vec<(Json, Json)> },
    Value { value: Json },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrecedenceCase {
    #[serde(default)]
    explicit: bool,
    #[serde(default)]
    router: bool,
    #[serde(default)]
    metadata: Params,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ForwardInput {
    vector_store_id: String,
    query: SearchQuery,
    optional_params: Params,
    api_base: String,
    litellm_params: Params,
    extra_body: Option<Params>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::QueryPreparation {
                queries,
                litellm_params,
            } => Ok(json!({
                "texts": queries.iter().map(query_text).collect::<Vec<_>>(),
                "cases": litellm_params.iter().map(preparation_case).collect::<Vec<_>>(),
            })),
            Input::ExecutorPrecedence { cases, forward } => {
                let kinds: Result<Vec<Value>, Box<HelperError>> =
                    cases.iter().map(precedence_case).collect();
                Ok(json!({
                    "cases": kinds?,
                    "sync": forward_report(forward, false)?,
                    "async": forward_report(forward, true)?,
                }))
            }
            Input::DirectStoreDefaults {
                api_bases,
                litellm_params,
            } => defaults_report(api_bases, litellm_params),
        }
    }
}

fn preparation_case(case: &PreparationCase) -> Value {
    let mut litellm_params = Params::new();
    if let Some(model) = &case.model {
        litellm_params.insert("litellm_embedding_model".to_owned(), model.clone());
    }
    let configuration = match &case.config {
        None => query_embedding_configuration(&litellm_params),
        Some(Slot::Entries { entries }) => embedding_configuration(entries.clone()),
        Some(Slot::Value { value }) => {
            litellm_params.insert("litellm_embedding_config".to_owned(), value.clone());
            query_embedding_configuration(&litellm_params)
        }
    };
    match (query_embedding_model(&litellm_params), configuration) {
        (Ok(model), Ok(prepared)) => {
            json!({"ok": true, "model": model, "configuration": prepared})
        }
        _ => json!({"ok": false}),
    }
}

fn precedence_case(case: &PrecedenceCase) -> Result<Value, Box<HelperError>> {
    let supplied: Option<&dyn VectorStoreEmbeddingExecutor> =
        case.explicit.then_some(&ExplicitExecutor);
    let handle = case.router.then(|| (RouterFixture, case.metadata.clone()));
    let selected = query_embedding_executor(supplied, handle, direct_executor()?);
    Ok(match selected {
        SelectedExecutor::Explicit(_) => json!({"kind": "explicit"}),
        SelectedExecutor::Router(router) => json!({"kind": "router", "metadata": router.metadata}),
        SelectedExecutor::Direct(_) => json!({"kind": "direct"}),
    })
}

fn forward_report(input: &ForwardInput, asynchronous: bool) -> crate::helpers::HelperResult {
    let mut logging = VectorStoreLogging::default();
    let request = QuerySearchRequest {
        search: SearchRequest {
            vector_store_id: &input.vector_store_id,
            query: &input.query,
            optional_params: &input.optional_params,
            api_base: &input.api_base,
            logging: &mut logging,
            litellm_params: &input.litellm_params,
            extra_body: input.extra_body.as_ref(),
        },
        embedding_executor: None,
    };
    let (url, body) = if asynchronous {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| fixtures::helper_error(error.to_string()))?;
        runtime
            .block_on(QueryFixture.transform_query_search_async(request))
            .map_err(|error| fixtures::helper_error(error.to_string()))?
    } else {
        QueryFixture
            .transform_query_search_request(request)
            .map_err(|error| fixtures::helper_error(error.to_string()))?
    };
    Ok(json!({
        "url": url,
        "body": body,
        "calls": logging.model_call_details.get("calls"),
    }))
}

fn defaults_report(
    api_bases: &[Option<String>],
    litellm_params: &Params,
) -> crate::helpers::HelperResult {
    let mut urls = Vec::with_capacity(api_bases.len());
    for api_base in api_bases {
        let url = DirectFixture
            .complete_url(api_base.as_deref(), &Params::new())
            .map_err(|error| fixtures::helper_error(error.to_string()))?;
        urls.push(url);
    }
    let query = SearchQuery::Text("hello".to_owned());
    let optional_params = Params::new();
    let mut logging = VectorStoreLogging::default();
    let response = VectorStoreResponse {
        status_code: 200,
        headers: BTreeMap::new(),
        body: Vec::new(),
    };
    let search_request = DirectFixture.transform_search_request(SearchRequest {
        vector_store_id: "vs_a",
        query: &query,
        optional_params: &optional_params,
        api_base: "",
        logging: &mut logging,
        litellm_params,
        extra_body: None,
    });
    Ok(json!({
        "urls": urls,
        "credentials": DirectFixture.auth_credentials(litellm_params),
        "endpoints": DirectFixture.endpoints_by_type(),
        "search_request": raised(search_request),
        "search_response": raised(DirectFixture.transform_search_response(&response, &mut logging)),
        "create_request": raised(DirectFixture.transform_create_request(&Params::new(), "")),
        "create_response": raised(DirectFixture.transform_create_response(&response)),
    }))
}

fn raised<T, E>(outcome: Result<T, E>) -> String
where
    E: std::error::Error,
{
    match outcome {
        Ok(_) => "no error".to_owned(),
        Err(error) => error.to_string(),
    }
}
