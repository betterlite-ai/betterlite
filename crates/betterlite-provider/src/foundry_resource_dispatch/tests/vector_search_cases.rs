use betterlite_foundry::vector_stores::environment::Params;
use betterlite_foundry::vector_stores::error::AzureSearchError;
use betterlite_foundry::vector_stores::reply::AzureSearchStoreReply;
use betterlite_foundry::vector_stores::search::{
    AzureSearchStoreRequest, SearchLogging, SearchQuery,
};
use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;

use super::fixture::{FixtureEmbedder, parse};
use crate::foundry_resource_dispatch::vector_search::{self, Call, Dispatch, Outcome};
use crate::foundry_resource_dispatch::{
    FoundryResourceError, FoundryResourceOperation, FoundryResourceResult, dispatch,
};

async fn run(call: Call<'_>) -> Result<FoundryResourceResult, FoundryResourceError> {
    dispatch(FoundryResourceOperation::VectorSearch(Box::new(Dispatch {
        call,
    })))
    .await
}

async fn outcome(call: Call<'_>) -> Outcome {
    match run(call).await {
        Ok(FoundryResourceResult::VectorSearch(outcome)) => outcome,
        Ok(_) | Err(_) => panic!("the vector search call returns its own outcome"),
    }
}

async fn leaf_error(call: Call<'_>) -> vector_search::Error {
    match run(call).await {
        Err(FoundryResourceError::VectorSearch(error)) => error,
        Ok(_) | Err(_) => panic!("the vector search call fails in its leaf"),
    }
}

async fn endpoint_rows() {
    let Outcome::Endpoints(found) = outcome(Call::Endpoints).await else {
        panic!("the endpoints call returns the index endpoints");
    };
    let read = found.read.unwrap();
    assert_eq!(read.len(), 5);
    assert_eq!(
        read.first(),
        Some(&(String::from("GET"), String::from("/indexes/")))
    );
    let write = found.write.unwrap();
    assert_eq!(write, [(String::from("POST"), String::from("/docs/index"))]);
}

async fn url_rows() {
    let empty = Params::new();
    let Outcome::Url(explicit) = outcome(Call::Url {
        api_base: Some("https://svc.search.windows.net/"),
        params: &empty,
    })
    .await
    else {
        panic!("the url call returns a url");
    };
    assert_eq!(explicit, "https://svc.search.windows.net");
    let named: Params = parse(r#"{"azure_search_service_name":"svc"}"#);
    let Outcome::Url(service) = outcome(Call::Url {
        api_base: Some(""),
        params: &named,
    })
    .await
    else {
        panic!("the url call returns a url");
    };
    assert_eq!(service, "https://svc.search.windows.net");
    let missing = leaf_error(Call::Url {
        api_base: None,
        params: &empty,
    })
    .await;
    assert!(matches!(
        missing,
        vector_search::Error::Search(AzureSearchError::MissingServiceName)
    ));
}

async fn auth_rows() {
    let params: Params = parse(r#"{"api_key":"search-secret"}"#);
    let Outcome::Auth(found) = outcome(Call::Auth { params: &params }).await else {
        panic!("the auth call returns the credential headers");
    };
    assert_eq!(
        found
            .headers
            .get("api-key")
            .map(|value| value.expose_secret().to_owned()),
        Some(String::from("search-secret"))
    );
    let missing = leaf_error(Call::Auth {
        params: &Params::new(),
    })
    .await;
    assert!(matches!(
        missing,
        vector_search::Error::Search(AzureSearchError::MissingApiKey)
    ));
}

fn search_request<'call>(
    query: &'call SearchQuery,
    optional: &'call Params,
    litellm: &'call Params,
    logging: &'call mut SearchLogging,
    embedder: &'call FixtureEmbedder,
) -> AzureSearchStoreRequest<'call> {
    AzureSearchStoreRequest {
        vector_store_id: "store-1",
        query,
        optional_params: optional,
        api_base: "https://svc.search.windows.net",
        litellm_params: litellm,
        logging,
        embedding_executor: embedder,
    }
}

async fn search_rows() {
    let litellm: Params = parse(
        r#"{"litellm_embedding_model":"openai/text-embedding-3-small","azure_search_vector_field":"vec"}"#,
    );
    let optional: Params = parse(r#"{"top_k":3}"#);
    let mut logging = SearchLogging::default();
    let embedder = FixtureEmbedder {
        vector: vec![0.5, 1.0],
    };
    let query = SearchQuery::Text(String::from("hello world"));
    let request = search_request(&query, &optional, &litellm, &mut logging, &embedder);
    let Outcome::Search(call) = outcome(Call::Search(request)).await else {
        panic!("the search call returns the upstream request");
    };
    assert_eq!(
        call.url,
        "https://svc.search.windows.net/indexes/store-1/docs/search?api-version=2024-07-01"
    );
    assert_eq!(call.body.top, 3);
    assert_eq!(call.body.select, "id,content");
    assert_eq!(call.body.vector_queries.len(), 1);
    assert_eq!(call.body.vector_queries[0].fields, "vec");
    assert_eq!(call.body.vector_queries[0].vector, [0.5_f64, 1.0]);
    assert_eq!(
        logging.model_call_details.get("input").map(|value| {
            match value {
                Json::Text(text) => text.as_str(),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    ""
                }
            }
        }),
        Some("hello world")
    );
}

async fn search_missing_model_row() {
    let no_model: Params = parse(r#"{"azure_search_vector_field":"vec"}"#);
    let optional: Params = parse(r#"{"top_k":3}"#);
    let mut logging = SearchLogging::default();
    let embedder = FixtureEmbedder {
        vector: vec![0.5, 1.0],
    };
    let query = SearchQuery::Text(String::from("hello world"));
    let request = search_request(&query, &optional, &no_model, &mut logging, &embedder);
    let missing = leaf_error(Call::Search(request)).await;
    assert!(matches!(
        missing,
        vector_search::Error::Search(AzureSearchError::MissingEmbeddingModel)
    ));
}

async fn reply_rows() {
    let headers = vec![(
        String::from("content-type"),
        String::from("application/json"),
    )];
    let logging = SearchLogging::default();
    let body = br#"{"value":[{"id":"d1","content":"hello","@search.score":0.25,"kept":"x"}]}"#;
    let Outcome::Reply(response) = outcome(Call::Reply(AzureSearchStoreReply {
        status_code: 200,
        headers: &headers,
        body: &body[..],
        logging: &logging,
    }))
    .await
    else {
        panic!("the reply call returns the projected search results");
    };
    assert_eq!(
        response.object,
        Some(String::from("vector_store.search_results.page"))
    );
    let results = response.data.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_id, Some(String::from("d1")));
    assert_eq!(results[0].filename, Some(String::from("Document d1")));

    let error = leaf_error(Call::Reply(AzureSearchStoreReply {
        status_code: 502,
        headers: &headers,
        body: b"bad gateway",
        logging: &SearchLogging::default(),
    }))
    .await;
    assert!(matches!(
        error,
        vector_search::Error::Search(AzureSearchError::ReplyJson { .. })
    ));
}

pub(super) async fn check() {
    endpoint_rows().await;
    url_rows().await;
    auth_rows().await;
    search_rows().await;
    search_missing_model_row().await;
    reply_rows().await;
}
