use betterlite_models::decisions::Json as ParamJson;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::{Json as BodyJson, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::claude_invoke::error::ClaudeInvokeError;
use crate::claude_invoke::request::{ClaudeInvokeRequest, TransformSources, build_base_request};

use super::MANTLE_PREFIX;

const MODEL_FIELD: &str = "model";
const STREAM_FIELD: &str = "stream";

// transform_request: strip mantle/ once, build B618's base body, then restore model and stream.
pub async fn transform_request<Source, Transport, Fetcher, Blocks, Ids, Lookup>(
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
    let model_id = request.model.replacen(MANTLE_PREFIX, "", 1);
    let stripped = ClaudeInvokeRequest {
        model: &model_id,
        messages: request.messages.clone(),
        optional_params: request.optional_params.clone(),
        litellm_params: request.litellm_params.clone(),
        headers: request.headers.clone(),
        defaults: request.defaults,
        catalog: request.catalog,
        default_chat_max_tokens: request.default_chat_max_tokens,
        modify_params: request.modify_params,
        global_drop_params: request.global_drop_params,
    };
    let mut body = build_base_request(&stripped, sources).await?.body;
    restore_body_fields(
        &mut body,
        &model_id,
        request.optional_params.get(STREAM_FIELD),
    );
    Ok(body)
}

// _restore_mantle_body_fields: the model overwritten unconditionally and stream set only for a literal true.
pub(super) fn restore_body_fields(body: &mut Object, model_id: &str, stream: Option<&ParamJson>) {
    body.insert(MODEL_FIELD.to_owned(), BodyJson::Text(model_id.to_owned()));
    if matches!(stream, Some(ParamJson::Bool(true))) {
        body.insert(STREAM_FIELD.to_owned(), BodyJson::Bool(true));
    }
}
