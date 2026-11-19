use betterlite_models::decisions::Json as ParamJson;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::{RemoteMediaFetcher, async_inline_remote_media};
use betterlite_prompts::media::parts::INLINE_EVERY_REMOTE_URL;
use betterlite_prompts::message_content::Message;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::claude_invoke::error::ClaudeInvokeError;
use crate::claude_invoke::request::{ClaudeInvokeRequest, TransformSources};

use super::request::transform_request;

// AmazonMantleConfig.async_transform_request: B183 inlines every remote part first, then the synchronous method runs.
pub async fn async_transform_request<Source, Transport, Fetcher, Blocks, Ids, Lookup>(
    request: &ClaudeInvokeRequest<'_>,
    sources: &mut TransformSources<'_, '_, Source, Transport, Fetcher, Blocks, Ids, Lookup>,
) -> Result<Object, ClaudeInvokeError<Blocks::Failure>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
    Lookup: ModelInfoLookup,
{
    let mut inlined = ClaudeInvokeRequest::clone(request);
    inlined.messages =
        inlined_messages::<Fetcher, Blocks>(&request.messages, sources.prompts.fetcher).await?;
    transform_request(&inlined, sources).await
}

async fn inlined_messages<Fetcher, Blocks>(
    messages: &[ParamJson],
    fetcher: &Fetcher,
) -> Result<Vec<ParamJson>, ClaudeInvokeError<Blocks::Failure>>
where
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
{
    let typed: Vec<Message> = serde_json::from_value(serde_json::to_value(messages)?)?;
    let inlined = async_inline_remote_media(&typed, &INLINE_EVERY_REMOTE_URL, fetcher).await?;
    let converted: Vec<ParamJson> = serde_json::from_value(serde_json::to_value(&inlined)?)?;
    Ok(converted)
}
