use crate::gemini_response_parts::content::{self, ThinkingBlock};
use crate::gemini_response_parts::media::{self, AudioPart, ImagePart};
use crate::gemini_response_parts::tools::{
    self, FunctionChunk, ServerToolInvocation, ToolCallChunk,
};
use betterlite_models::decisions::Json;
use betterlite_openai::stream::chunk::{Logprobs, TokenLogprob, TopLogprob};
use serde::{Deserialize, Serialize};
use serde_json::Number;
use thiserror::Error;
use uuid::Uuid;
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Part {
    pub text: Option<String>,
    pub inline_data: Option<InlineData>,
    pub function_call: Option<FunctionCall>,
    pub tool_call: Option<ServerTool>,
    pub tool_response: Option<ServerTool>,
    pub thought: Option<bool>,
    pub thought_signature: Option<String>,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InlineData {
    pub mime_type: String,
    pub data: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub args: Option<Json>,
    pub id: Option<String>,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerTool {
    pub tool_type: Option<String>,
    pub id: Option<String>,
    pub args: Option<Json>,
    pub response: Option<Json>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogprobsResult {
    pub chosen_candidates: Option<Vec<LogprobCandidate>>,
    pub top_candidates: Option<Vec<TopCandidates>>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogprobCandidate {
    pub token: String,
    pub log_probability: Number,
}
#[derive(Debug, Clone, Deserialize)]
pub struct TopCandidates {
    pub candidates: Vec<LogprobCandidate>,
}
#[derive(Debug, Error)]
pub enum PartsError {
    #[error("the function call arguments cannot be encoded")]
    Arguments {
        #[source]
        source: serde_json::Error,
    },
    #[error("the audio expiry cannot be computed from {now} seconds")]
    Expiry { now: i64 },
    #[error("the tool call index overflows")]
    ToolIndex,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallMode {
    LegacyFunction,
    Tools,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartContext {
    pub cumulative_tool_call_index: u32,
    pub call_mode: CallMode,
    pub now: i64,
}
#[derive(Debug, Serialize)]
pub struct PartsProjection {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub thinking_blocks: Vec<ThinkingBlock>,
    pub thought_signatures: Option<Vec<String>>,
    pub server_side_tool_invocations: Option<Vec<ServerToolInvocation>>,
    pub images: Vec<ImagePart>,
    pub audio: Option<AudioPart>,
    pub function: Option<FunctionChunk>,
    pub tool_calls: Option<Vec<ToolCallChunk>>,
    pub next_tool_call_index: u32,
    pub logprobs: Option<Logprobs>,
}
pub fn project_parts(
    parts: &[Part],
    logprobs: Option<&LogprobsResult>,
    context: PartContext,
    ids: &mut dyn FnMut() -> Uuid,
) -> Result<PartsProjection, PartsError> {
    let audio = media::audio_part(parts, context.now, ids)?;
    let (text, reasoning_content) = content::assistant_content(parts);
    let (function, tool_calls, next_tool_call_index) = tools::transform(
        parts,
        context.cumulative_tool_call_index,
        context.call_mode,
        ids,
    )?;
    Ok(PartsProjection {
        content: text,
        reasoning_content,
        thinking_blocks: content::thinking_blocks(parts),
        thought_signatures: content::thought_signatures(parts),
        server_side_tool_invocations: tools::invocations(parts),
        images: media::image_parts(parts),
        audio,
        function,
        tool_calls,
        next_tool_call_index,
        logprobs: transform_logprobs(logprobs),
    })
}
fn transform_logprobs(source: Option<&LogprobsResult>) -> Option<Logprobs> {
    let result = source?;
    let chosen = result.chosen_candidates.as_ref()?;
    let top_candidates = result.top_candidates.as_deref();
    let content = chosen
        .iter()
        .enumerate()
        .map(|(index, candidate)| TokenLogprob {
            token: candidate.token.clone(),
            logprob: candidate.log_probability.clone(),
            bytes: None,
            top_logprobs: top_candidates
                .and_then(|tops| tops.get(index))
                .into_iter()
                .flat_map(|top| &top.candidates)
                .map(|option| TopLogprob {
                    token: option.token.clone(),
                    logprob: option.log_probability.clone(),
                    bytes: None,
                })
                .collect(),
        })
        .collect();
    Some(Logprobs {
        content: Some(content),
        refusal: None,
    })
}
