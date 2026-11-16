use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::media::inline::RemoteMediaFetcher;

use crate::bedrock_model_capabilities::endpoints::supports_openai_responses;
use crate::model_routes::{CHAT_COMPLETIONS_PREFIX, ParamMap};
use crate::responses::error::ResponsesError;

pub const CUSTOM_LLM_PROVIDER: ProviderId = ProviderId::Bedrock;

pub const SUPPORTS_NATIVE_FILE_SEARCH: bool = false;

pub struct BedrockResponsesConfig<Source> {
    pub fetcher: Source,
}

impl<Source> BedrockResponsesConfig<Source>
where
    Source: RemoteMediaFetcher,
{
    pub fn for_model(
        model: Option<&str>,
        model_cost: &ParamMap,
        fetcher: Source,
    ) -> Result<Option<Self>, ResponsesError> {
        let Some(name) = model else {
            return Ok(None);
        };
        let routed = name.strip_prefix(CHAT_COMPLETIONS_PREFIX).unwrap_or(name);
        let supported = supports_openai_responses(Some(routed), model_cost)?;
        Ok(supported.then_some(Self { fetcher }))
    }
}
