use std::str::FromStr;
use std::time::Duration;

use betterlite_bedrock::rerank::plan::RerankEnvironment;
use betterlite_bedrock::transcription::error::CallError;
use betterlite_openai::api::rerank_contracts::records::{RerankRequest, RerankResponse};
use betterlite_provider::bedrock_other_dispatch::BedrockOtherResult;
use betterlite_provider::bedrock_other_dispatch::call::{BedrockOtherOperation, RerankOperation};
use betterlite_provider::bedrock_other_dispatch::test_support::binding::Recording;
use serde_json::{Number, Value, json};

use super::super::harness::{Harness, Outcome, exchange, now};
use super::super::model::{Failed, Probe, Unavailable, api_key, extra_pairs, param_map};
use super::row;
use crate::openai_media_dispatch::failure;

const GENERATED_ID: &str = "00000000-0000-4000-8000-000000000000";

pub(super) async fn rerank_row(probe: &Probe) -> Result<Value, Failed> {
    let extra = extra_pairs(probe);
    let key = api_key(probe);
    let operation = BedrockOtherOperation::Rerank(RerankOperation {
        request: request(probe),
        optional_params: param_map(probe)?,
        api_base: probe.api_base.as_deref(),
        extra_headers: extra.as_deref(),
        api_key: key.as_ref(),
        timeout: probe.timeout_ms.map(Duration::from_millis),
        is_async: false,
        environment: RerankEnvironment::default(),
        now: now()?,
        generated_id: GENERATED_ID,
    });
    let harness = Harness::new(probe)?;
    let outcome: Outcome<CallError> =
        exchange::<Unavailable, Recording>(&harness, operation).await?;
    let value = match &outcome.result {
        Ok(BedrockOtherResult::Rerank(response)) => rerank_value(response)?,
        Ok(_other) => Value::Null,
        Err(_reported) => Value::Null,
    };
    row(&outcome, value, None)
}

fn request(probe: &Probe) -> RerankRequest {
    RerankRequest {
        model: probe.model.clone(),
        query: probe.query.clone().unwrap_or_default(),
        top_n: probe.top_n,
        documents: probe.documents.clone(),
        rank_fields: probe.rank_fields.clone(),
        return_documents: probe.return_documents,
        max_chunks_per_doc: None,
        max_tokens_per_doc: None,
        instruction: None,
        truncate_prompt_tokens: None,
        truncation_side: None,
        max_tokens_per_query: None,
    }
}

// The reply's own id and its results in the provider's order, each under its original index
fn rerank_value(response: &RerankResponse) -> Result<Value, Failed> {
    let results = match &response.results {
        None => Value::Null,
        Some(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                let score = Number::from_str(&item.relevance_score.0.to_string())
                    .map_err(|error| failure("Score", error))?;
                values.push(json!({"index": item.index, "score": score}));
            }
            Value::Array(values)
        }
    };
    Ok(json!({"id": &response.id, "results": results}))
}
