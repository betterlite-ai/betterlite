use betterlite_foundry::vector_stores::environment::Params;
use betterlite_foundry::vector_stores::error::AzureSearchError;
use betterlite_foundry::vector_stores::reply::{AzureSearchStoreReply, decode_azure_search_reply};
use betterlite_foundry::vector_stores::search::{
    AzureSearchStoreRequest, SearchLogging, search_request,
};
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreResultContent, VectorStoreSearchResponse, VectorStoreSearchResult,
};
use rust_decimal::prelude::ToPrimitive;
use serde_json::{Number, Value, json};

use super::super::fixtures::FixtureExecutor;
use super::super::scenarios::{CitationScenario, DocumentScoreCitationInput};

pub(super) fn citation_report(input: &DocumentScoreCitationInput) -> Value {
    json!({
        "responses": input.responses.iter().map(citation_outcome).collect::<Vec<Value>>(),
    })
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
