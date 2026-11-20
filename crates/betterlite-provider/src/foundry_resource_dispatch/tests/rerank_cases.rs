use std::collections::BTreeMap;

use betterlite_foundry::rerank::error::FoundryRerankError;
use betterlite_foundry::rerank::request::FoundryRerankRequest;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;

use super::fixture::parse;
use crate::foundry_resource_dispatch::rerank::{self, Call, Dispatch, Outcome};
use crate::foundry_resource_dispatch::{
    FoundryResourceError, FoundryResourceOperation, FoundryResourceResult, dispatch,
};

fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0010_0000,
        max_items: 64,
    }
}

async fn run(call: Call<'_>) -> Result<FoundryResourceResult, FoundryResourceError> {
    dispatch(FoundryResourceOperation::Rerank(Box::new(Dispatch {
        call,
    })))
    .await
}

async fn outcome(call: Call<'_>) -> Outcome {
    match run(call).await {
        Ok(FoundryResourceResult::Rerank(outcome)) => outcome,
        Ok(_) | Err(_) => panic!("the rerank call returns its own outcome"),
    }
}

async fn leaf_error(call: Call<'_>) -> rerank::DispatchError {
    match run(call).await {
        Err(FoundryResourceError::Rerank(error)) => error,
        Ok(_) | Err(_) => panic!("the rerank call fails in its leaf"),
    }
}

async fn url_rows() {
    let Outcome::Url(url) = outcome(Call::Url {
        api_base: Some("https://res.services.ai.azure.com"),
    })
    .await
    else {
        panic!("the url call returns a url");
    };
    assert_eq!(url, "https://res.services.ai.azure.com/v1/rerank");
    let Outcome::Url(versioned) = outcome(Call::Url {
        api_base: Some("https://res.services.ai.azure.com/models/"),
    })
    .await
    else {
        panic!("the url call returns a url");
    };
    assert_eq!(
        versioned,
        "https://res.services.ai.azure.com/models/v1/rerank"
    );
    let missing = leaf_error(Call::Url { api_base: None }).await;
    assert!(matches!(
        missing,
        rerank::DispatchError::Rerank(FoundryRerankError::MissingApiBase)
    ));
}

async fn request_rows() {
    let request = FoundryRerankRequest {
        model: String::from("cohere-rerank-v3-multilingual"),
        query: String::from("what is rust"),
        documents: vec![
            RerankDocument::Text(String::from("rust is a language")),
            RerankDocument::Text(String::from("a crab")),
        ],
        top_n: Some(2),
        rank_fields: None,
        return_documents: None,
        max_chunks_per_doc: None,
    };
    let Outcome::Request(params) = outcome(Call::Request(&request)).await else {
        panic!("the request call returns the rerank body");
    };
    assert_eq!(
        params.get("query"),
        Some(&Json::Text(String::from("what is rust")))
    );
    assert_eq!(
        params.get("model"),
        Some(&Json::Text(String::from("cohere-rerank-v3-multilingual")))
    );
    let top_n: Json = parse("2");
    assert_eq!(params.get("top_n"), Some(&top_n));
    assert_eq!(params.get("return_documents"), Some(&Json::Bool(true)));
    let Some(Json::List(documents)) = params.get("documents") else {
        panic!("the rerank body carries its documents");
    };
    assert_eq!(documents.len(), 2);
}

async fn reply_rows() {
    let hidden: BTreeMap<String, Json> =
        parse(r#"{"llm_provider-azureml-model-group":"offer-cohere-rerank-mul-paygo"}"#);
    let body = br#"{"id":"r1","results":[{"index":2,"relevance_score":0.5},{"index":0,"relevance_score":0.9}],"meta":{"billed_units":{"search_units":1}}}"#;
    let Outcome::Reply(reply) = outcome(Call::Reply {
        status: 200,
        body: &body[..],
        limits: Box::new(limits()),
        hidden_params: Box::new(hidden),
    })
    .await
    else {
        panic!("the reply call returns the rerank reply");
    };
    let results = reply.results.unwrap();
    let indices: Vec<i64> = results.iter().map(|result| result.index).collect();
    assert_eq!(indices, [2_i64, 0]);
    let expected: BTreeMap<String, Json> = parse(
        r#"{"llm_provider-azureml-model-group":"offer-cohere-rerank-mul-paygo","model":"azure_ai/cohere-rerank-v3-multilingual"}"#,
    );
    assert_eq!(reply.hidden_params, expected);
    assert!(reply.meta.is_some());

    let error = leaf_error(Call::Reply {
        status: 502,
        body: b"bad gateway",
        limits: Box::new(limits()),
        hidden_params: Box::new(BTreeMap::new()),
    })
    .await;
    assert!(matches!(
        error,
        rerank::DispatchError::Rerank(FoundryRerankError::Cohere(_))
    ));
}

pub(super) async fn check() {
    url_rows().await;
    request_rows().await;
    reply_rows().await;
}
