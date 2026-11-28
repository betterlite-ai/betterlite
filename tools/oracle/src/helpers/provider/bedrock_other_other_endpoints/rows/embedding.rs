use betterlite_bedrock::bedrock_embedding_io::transport::Timeout;
use betterlite_bedrock::embedding_call::{EmbeddingCallResponse, EmbeddingEnvironment};
use betterlite_bedrock::transcription::error::CallError;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingRecord;
use betterlite_openai::embedding::EmbeddingRecord;
use betterlite_provider::bedrock_other_dispatch::BedrockOtherResult;
use betterlite_provider::bedrock_other_dispatch::call::{
    BedrockOtherOperation, EmbeddingInput, EmbeddingOperation,
};
use betterlite_provider::bedrock_other_dispatch::test_support::binding::Recording;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::super::harness::{Handler, Harness, Outcome, exchange};
use super::super::model::{Failed, Probe, extra_pairs, param_map};
use super::row;

pub(super) async fn embedding_row(probe: &Probe) -> Result<Value, Failed> {
    let extra = extra_pairs(probe);
    let key = probe.api_key.clone().map(SecretString::from);
    let bearer = probe
        .environment
        .get("AWS_BEARER_TOKEN_BEDROCK")
        .cloned()
        .map(SecretString::from);
    let operation = BedrockOtherOperation::Embedding(EmbeddingOperation {
        model: &probe.model,
        input: EmbeddingInput::Multiple(&probe.input),
        optional_params: param_map(probe)?,
        api_base: probe.api_base.as_deref(),
        extra_headers: extra.as_deref(),
        api_key: key.as_ref(),
        timeout: probe.timeout.map(Timeout::Seconds),
        aembedding: false,
        drop_params: false,
        environment: EmbeddingEnvironment {
            variables: &probe.environment,
            runtime_endpoint: probe
                .environment
                .get("AWS_BEDROCK_RUNTIME_ENDPOINT")
                .map(String::as_str),
            bearer_token: bearer.as_ref(),
        },
    });
    let harness = Harness::new(probe)?;
    let outcome: Outcome<CallError> = exchange::<Handler, Recording>(&harness, operation).await?;
    let value = match &outcome.result {
        Ok(BedrockOtherResult::Embedding(response)) => embedding_value(response),
        Ok(_other) => Value::Null,
        Err(_reported) => Value::Null,
    };
    row(&outcome, value, None)
}

// The reply fields both dialects carry: each input's vector under its own index, with the usage totals
fn embedding_value(response: &EmbeddingCallResponse) -> Value {
    match response {
        EmbeddingCallResponse::Batch(reply) => json!({
            "model": &reply.model,
            "data": batch_records(&reply.data),
            "usage": {
                "prompt_tokens": reply.usage.prompt_tokens,
                "total_tokens": reply.usage.total_tokens,
            },
        }),
        EmbeddingCallResponse::Legacy(reply) => json!({
            "model": &reply.model,
            "data": legacy_records(&reply.data),
            "usage": {
                "prompt_tokens": reply.usage.prompt_tokens,
                "total_tokens": reply.usage.total_tokens,
            },
        }),
    }
}

fn batch_records(data: &[EmbeddingRecord]) -> Value {
    let mut values = Vec::with_capacity(data.len());
    for record in data {
        values.push(json!({"index": record.index, "embedding": &record.embedding}));
    }
    Value::Array(values)
}

fn legacy_records(data: &[LegacyEmbeddingRecord]) -> Value {
    let mut values = Vec::with_capacity(data.len());
    for record in data {
        values.push(json!({"index": record.index, "embedding": &record.embedding}));
    }
    Value::Array(values)
}
