use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_with::skip_serializing_none;
use thiserror::Error;

use crate::chat::response::ChatUsage;

#[derive(Debug, Error)]
pub enum ChunkError {
    #[error("the provider stream chunk is invalid: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Option<ChatUsage>,
    pub system_fingerprint: Option<String>,
    pub service_tier: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub index: u32,
    pub delta: Delta,
    pub finish_reason: Option<String>,
    pub logprobs: Option<Logprobs>,
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Logprobs {
    pub content: Option<Vec<TokenLogprob>>,
    pub refusal: Option<Vec<TokenLogprob>>,
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenLogprob {
    pub token: String,
    pub logprob: Number,
    pub bytes: Option<Vec<u8>>,
    pub top_logprobs: Vec<TopLogprob>,
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopLogprob {
    pub token: String,
    pub logprob: Number,
    pub bytes: Option<Vec<u8>>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Delta {
    pub role: Option<String>,
    pub content: Option<String>,
    pub refusal: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub function_call: Option<Function>,
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub index: u32,
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub function: Option<Function>,
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Function {
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: String,
}

impl Chunk {
    pub fn parse(
        bytes: &[u8],
        model: &str,
        include_usage: bool,
    ) -> Result<Option<Self>, ChunkError> {
        let mut chunk: Self = serde_json::from_slice(bytes)?;
        if !model.is_empty() {
            model.clone_into(&mut chunk.model);
        }
        if chunk.choices.is_empty() {
            if !include_usage {
                return Ok(None);
            }
            chunk.choices.push(Choice {
                index: 0,
                delta: Delta::default(),
                finish_reason: None,
                logprobs: None,
            });
        }
        if !include_usage {
            chunk.usage = None;
        }
        if let Some(usage) = &mut chunk.usage {
            usage.completion_tokens_details.get_or_insert_with(|| {
                BTreeMap::from([
                    (String::from("reasoning_tokens"), Some(0)),
                    (String::from("text_tokens"), Some(usage.completion_tokens)),
                ])
            });
        }
        Ok(Some(chunk))
    }
}

mod tests;
