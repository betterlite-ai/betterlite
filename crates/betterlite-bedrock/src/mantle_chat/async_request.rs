use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::claude_invoke::error::ClaudeInvokeError;
use crate::claude_invoke::request::{ClaudeInvokeRequest, TransformSources, inlined_messages};

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
