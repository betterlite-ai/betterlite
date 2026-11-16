use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::response::reply::Usage;
use crate::databricks_chat::response::usage::usage_from;
use crate::databricks_chat::stream::choice::StreamChoice;
use crate::databricks_chat::stream::wire::DatabricksChunk;
use crate::ollama_chat::ids::IdSource;

pub mod choice;
pub mod delta;
mod tests;
pub mod wire;

// ModelResponseStream as DatabricksChatResponseIterator builds it for one chunk
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StreamChunk {
    pub id: String,
    pub created: i64,
    pub model: Option<String>,
    pub object: String,
    pub choices: Vec<StreamChoice>,
    pub provider_specific_fields: Option<Object>,
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
}

// DatabricksChatResponseIterator: the chunk parser keeps the last function name it saw in json mode
#[derive(Debug, Default)]
pub struct ChunkParser {
    json_mode: bool,
    last_function_name: Option<String>,
}

impl ChunkParser {
    #[must_use]
    pub const fn new(json_mode: bool) -> Self {
        Self {
            json_mode,
            last_function_name: None,
        }
    }

    // chunk_parser over one decoded data line, reading the choices first as LiteLLM does
    pub fn parse(
        &mut self,
        chunk: &str,
        ids: &dyn IdSource,
    ) -> Result<StreamChunk, DatabricksChatError> {
        let parsed: DatabricksChunk =
            serde_json::from_str(chunk).map_err(DatabricksChatError::Decode)?;
        let Some(given_choices) = parsed.choices.required("choices", chunk)? else {
            return Err(DatabricksChatError::Shape {
                message: String::from("the choices of a Databricks stream chunk are null"),
            });
        };
        let mut choices = Vec::with_capacity(given_choices.len());
        for given in given_choices {
            choices.push(choice::transform(given, self, chunk)?);
        }
        let service_tier = match parsed.service_tier {
            Some(Json::Text(tier)) if !tier.is_empty() => Some(tier),
            None | Some(_) => None,
        };
        Ok(StreamChunk {
            id: parsed
                .id
                .required("id", chunk)?
                .unwrap_or_else(|| ids.next_id()),
            created: parsed
                .created
                .required("created", chunk)?
                .ok_or(DatabricksChatError::NullCreated)?,
            model: parsed.model.required("model", chunk)?,
            object: String::from("chat.completion.chunk"),
            choices,
            provider_specific_fields: None,
            usage: parsed.usage.as_ref().map(usage_from).transpose()?,
            service_tier,
        })
    }
}
