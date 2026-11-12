use async_trait::async_trait;
use betterlite_openai::embedding::{
    CompletionTokensDetails, EmbeddingResponse, PromptTokensDetails,
};
use betterlite_provider::ollama_generate::embed::{
    OllamaEmbeddingRequest, TokenEncoder, embed_url, transform_embedding_request,
    transform_embedding_response,
};
use betterlite_tokenizer::count::error::CountError;
use serde::Serialize;
use serde_json::{Value, json};

use super::EmbeddingCase;
use super::EncoderKind;
use super::ops::{failure, runtime};
use crate::helpers::HelperResult;

struct WordsEncoder;

#[async_trait]
impl TokenEncoder for WordsEncoder {
    async fn encode(&self, text: &str) -> Result<Vec<u32>, CountError> {
        let count = text.split_whitespace().count();
        Ok(vec![1_u32; count])
    }
}

struct FailingEncoder;

#[async_trait]
impl TokenEncoder for FailingEncoder {
    async fn encode(&self, _text: &str) -> Result<Vec<u32>, CountError> {
        Err(CountError::Overflow)
    }
}

#[derive(Serialize)]
struct EmbeddingAnswer<'response> {
    object: &'response str,
    model: &'response str,
    data: Vec<Value>,
    usage: [u32; 3],
    prompt_tokens_details: Option<PromptTokensDetails>,
    completion_tokens_details: Option<CompletionTokensDetails>,
}

pub(super) fn embedding_cases(cases: &[EmbeddingCase]) -> HelperResult {
    let runtime = runtime()?;
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        found.push(runtime.block_on(embedding_case(case)));
    }
    Ok(json!({"cases": found}))
}

async fn embedding_case(case: &EmbeddingCase) -> Value {
    let url = embed_url(&case.api_base);
    let payload: OllamaEmbeddingRequest =
        match transform_embedding_request(&case.model, &case.prompts, &case.options) {
            Ok(payload) => payload,
            Err(error) => return failure(&error),
        };
    let words = WordsEncoder;
    let failing = FailingEncoder;
    let encoder: Option<&dyn TokenEncoder> = match case.encoder {
        EncoderKind::None => None,
        EncoderKind::Words => Some(&words),
        EncoderKind::Failing => Some(&failing),
    };
    match transform_embedding_response(
        case.status,
        case.body.as_bytes(),
        &case.model,
        &case.prompts,
        encoder,
    )
    .await
    {
        Ok(response) => json!({
            "ok": true,
            "url": url,
            "payload": payload,
            "reply": embedding_value(&response),
        }),
        Err(error) => failure(&error),
    }
}

fn embedding_value(response: &EmbeddingResponse) -> Value {
    let data: Vec<Value> = response
        .data
        .iter()
        .map(|record| {
            json!({
                "object": record.object,
                "index": record.index,
                "embedding": serde_json::to_value(&record.embedding).unwrap_or(Value::Null),
            })
        })
        .collect();
    let answer = EmbeddingAnswer {
        object: &response.object,
        model: &response.model,
        data,
        usage: [
            response.usage.prompt_tokens,
            response.usage.completion_tokens,
            response.usage.total_tokens,
        ],
        prompt_tokens_details: response.usage.prompt_tokens_details.clone(),
        completion_tokens_details: response.usage.completion_tokens_details.clone(),
    };
    serde_json::to_value(answer).unwrap_or(Value::Null)
}
