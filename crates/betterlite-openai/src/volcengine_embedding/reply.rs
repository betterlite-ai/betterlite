use serde::Deserialize;

use crate::embedding::{
    CompletionTokensDetails, EmbeddingRecord, EmbeddingResponse, EmbeddingUsage,
    PromptTokensDetails,
};
use crate::openai_base_config::Headers;
use crate::volcengine_common::VolcEngineError;

// LiteLLM raises a bare exception on a reply it cannot read, which its proxy answers with a 500
const FAILURE_STATUS: u16 = 500;

#[derive(Debug, Deserialize)]
struct WireReply {
    #[serde(default)]
    data: Option<Vec<EmbeddingRecord>>,
    // the outer option is a missing key and the inner one an explicit null, which LiteLLM reports as null
    #[serde(default, with = "serde_with::rust::double_option")]
    model: Option<Option<String>>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

// the counters default to zero and the two detail records stay absent, as LiteLLM's Usage() does
#[derive(Debug, Default, Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    #[serde(default)]
    total_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens_details: Option<CompletionTokensDetails>,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
}

impl WireUsage {
    fn into_usage(self) -> EmbeddingUsage {
        EmbeddingUsage {
            prompt_tokens: self.prompt_tokens.unwrap_or_default(),
            completion_tokens: self.completion_tokens.unwrap_or_default(),
            completion_tokens_details: self.completion_tokens_details,
            prompt_tokens_details: self.prompt_tokens_details,
            total_tokens: self.total_tokens.unwrap_or_default(),
        }
    }
}

// the reply's id and object are dropped, and a missing model falls back to the requested one
pub fn decode_reply(raw: &[u8], model: &str) -> Result<EmbeddingResponse, VolcEngineError> {
    let wire: WireReply = serde_json::from_slice(raw).map_err(|source| {
        malformed(format!(
            "the Volcengine embedding reply is malformed: {source}"
        ))
    })?;
    let reply_model = match wire.model {
        None => model.to_owned(),
        Some(Some(text)) => text,
        Some(None) => {
            return Err(malformed(String::from(
                "the Volcengine embedding reply has a model that is not a string",
            )));
        }
    };
    Ok(EmbeddingResponse {
        object: String::from("list"),
        data: wire.data.unwrap_or_default(),
        model: reply_model,
        usage: wire.usage.unwrap_or_default().into_usage(),
    })
}

fn malformed(message: String) -> VolcEngineError {
    VolcEngineError {
        status_code: FAILURE_STATUS,
        message,
        headers: Headers::new(),
    }
}
