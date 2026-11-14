pub mod error;
pub mod input;
pub mod settings;

mod tests;

use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::message_content::Message;

use crate::gemini_history::convert::{History, convert_messages_with_history};
use crate::gemini_history::error::HistoryError;
use crate::gemini_options::params::{ReasoningSupport, THINKING_PARAMS};
use crate::model_urls::model_name::is_gemini_3_or_newer;

// LiteLLM advertises these names in this order, without the Vertex-only top_k, top_logprobs and extra_headers
pub const SUPPORTED_OPENAI_PARAMS: [&str; 20] = [
    "temperature",
    "top_p",
    "max_tokens",
    "max_completion_tokens",
    "stream",
    "tools",
    "tool_choice",
    "functions",
    "response_format",
    "n",
    "stop",
    "logprobs",
    "frequency_penalty",
    "presence_penalty",
    "seed",
    "modalities",
    "parallel_tool_calls",
    "web_search_options",
    "include_server_side_tool_invocations",
    "service_tier",
];

pub const AUDIO_PARAM: &str = "audio";
const TTS_MARKER: &str = "tts";

#[must_use]
pub fn supported_openai_params(model: &str, support: &dyn ReasoningSupport) -> Vec<&'static str> {
    let mut params = SUPPORTED_OPENAI_PARAMS.to_vec();
    if is_gemini_3_or_newer(model) || support.supports_reasoning(model) {
        params.extend(THINKING_PARAMS);
    }
    // the audio capability follows the model name as written, not the catalog's audio modes
    if model.contains(TTS_MARKER) {
        params.push(AUDIO_PARAM);
    }
    params
}

pub async fn transform_messages<Source>(
    messages: &[Message],
    model: &str,
    fetcher: &Source,
) -> Result<History, HistoryError>
where
    Source: RemoteMediaFetcher + Sync,
{
    input::reject_remote_media(messages)?;
    convert_messages_with_history(messages, Some(model), fetcher).await
}
