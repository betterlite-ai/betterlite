use std::str;

use betterlite_openai::embedding::{
    EmbeddingRecord, EmbeddingResponse, EmbeddingUsage, EmbeddingVector,
};
use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;
use serde_json::Number;

use crate::oci_common::OciError;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OciEmbedResponse {
    #[serde(default)]
    pub id: Option<String>,
    pub embeddings: Vec<Vec<Number>>,
    pub model_id: String,
    pub model_version: String,
    #[serde(default)]
    pub input_text_token_counts: Option<Vec<u32>>,
    #[serde(default)]
    pub usage: Option<OciEmbedUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OciEmbedUsage {
    pub prompt_tokens: u32,
    pub total_tokens: u32,
}

pub fn decode_embed_response(status: u16, body: &[u8]) -> Result<EmbeddingResponse, OciError> {
    if status != 200 {
        return Err(OciError {
            status,
            message: body_text(body),
        });
    }
    let document: Json = serde_json::from_slice(body).map_err(|source| OciError {
        status,
        message: format!("Failed to parse OCI embed response as JSON: {source}"),
    })?;
    // the derived decoder also accepts a struct as a field sequence, which the OCI body never is
    let Json::Object(fields) = &document else {
        return Err(schema_mismatch("the response body is not a JSON object"));
    };
    if fields
        .get("usage")
        .is_some_and(|usage| !matches!(usage, Json::Null | Json::Object(_)))
    {
        return Err(schema_mismatch("usage is not a JSON object"));
    }
    let parsed: OciEmbedResponse = serde_json::from_slice(body).map_err(|source| OciError {
        status: 500,
        message: format!("OCI embed response does not match expected schema: {source}"),
    })?;
    embed_response(parsed)
}

fn embed_response(parsed: OciEmbedResponse) -> Result<EmbeddingResponse, OciError> {
    let usage = response_usage(&parsed)?;
    let row_count = parsed.embeddings.len();
    let mut data = Vec::with_capacity(row_count);
    for (position, embedding) in parsed.embeddings.into_iter().enumerate() {
        let index = u32::try_from(position).map_err(|_source| OciError {
            status: 500,
            message: "OCI embed response does not match expected schema: too many embedding rows"
                .to_owned(),
        })?;
        data.push(EmbeddingRecord {
            object: "embedding".to_owned(),
            embedding: EmbeddingVector::Floats(embedding),
            index,
        });
    }
    Ok(EmbeddingResponse {
        object: "list".to_owned(),
        data,
        model: parsed.model_id,
        usage,
    })
}

fn schema_mismatch(detail: &str) -> OciError {
    OciError {
        status: 500,
        message: format!("OCI embed response does not match expected schema: {detail}"),
    }
}

fn body_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_| {
            format!(
                "the response body holds {} bytes that are not UTF-8",
                body.len()
            )
        },
        str::to_owned,
    )
}

fn response_usage(parsed: &OciEmbedResponse) -> Result<EmbeddingUsage, OciError> {
    let (prompt_tokens, total_tokens) = match (&parsed.input_text_token_counts, parsed.usage) {
        (Some(counts), _) => {
            let total = token_count_sum(counts)?;
            (total, total)
        }
        (None, Some(usage)) => (usage.prompt_tokens, usage.total_tokens),
        (None, None) => (0, 0),
    };
    Ok(EmbeddingUsage {
        prompt_tokens,
        completion_tokens: 0,
        completion_tokens_details: None,
        prompt_tokens_details: None,
        total_tokens,
    })
}

fn token_count_sum(counts: &[u32]) -> Result<u32, OciError> {
    let mut total = 0_u32;
    for count in counts {
        total = total.checked_add(*count).ok_or_else(|| OciError {
            status: 500,
            message: "OCI embed response does not match expected schema: the token counts overflow"
                .to_owned(),
        })?;
    }
    Ok(total)
}
