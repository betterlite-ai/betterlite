use std::error::Error as StdError;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::capabilities::effort_param::model_supports_effort_param;
use betterlite_anthropic::request::{AnthropicRequest, BillingHeader, PreparedRequest};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json as ParamJson;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::{
    CapabilityEnvironment, CapabilityPredicates as _, CapabilityRequest,
};
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::{Json as BodyJson, Object};
use betterlite_prompts::media::inline::{RemoteMediaFetcher, async_inline_remote_media};
use betterlite_prompts::media::parts::INLINE_EVERY_REMOTE_URL;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;

use crate::aws_settings::AUTHENTICATION_PARAMS;
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::bedrock_model_capabilities::effort::normalize_opus_effort;
use crate::claude_invoke::error::ClaudeInvokeError;
use crate::claude_invoke::request::{ClaudeInvokeRequest, TransformSources};
use crate::claude_invoke::tools::normalize_tool_search_tools;
use crate::claude_invoke::{ANTHROPIC_VERSION, CUSTOM_LLM_PROVIDER, STRIPS_BILLING_METADATA};
use crate::invoke_schema::{
    InvokeSchemaCapabilities, apply_structured_output, normalize_tool_custom_field,
    normalize_tool_schema_types, strip_eager_input_streaming, strip_unsupported_config_keys,
};
use crate::model_routes::ParamValue;

use super::MANTLE_PREFIX;

const ANTHROPIC_VERSION_FIELD: &str = "anthropic_version";
const MODEL_FIELD: &str = "model";
const OUTPUT_CONFIG_FIELD: &str = "output_config";
const STREAM_FIELD: &str = "stream";
const TOOLS_FIELD: &str = "tools";

// AmazonMantleConfig.transform_request: the model ID loses its first mantle/ prefix, the parent's base builder runs
// without its document conversion or computed beta list, and the copy puts the model back.
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
    let mut body = base_request(&model_id, request, sources).await?;
    restore_body_fields(
        &mut body,
        &model_id,
        request.optional_params.get(STREAM_FIELD),
    );
    Ok(body)
}

// AmazonMantleConfig._restore_mantle_body_fields: a copy with the model overwritten unconditionally, stream set only
// for a literal true, and every other field (a caller's stream included) left as it was.
pub(super) fn restore_body_fields(body: &mut Object, model_id: &str, stream: Option<&ParamJson>) {
    body.insert(MODEL_FIELD.to_owned(), BodyJson::Text(model_id.to_owned()));
    if matches!(stream, Some(ParamJson::Bool(true))) {
        body.insert(STREAM_FIELD.to_owned(), BodyJson::Bool(true));
    }
}

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
    inlined.messages = inlined_messages(&request.messages, sources.prompts.fetcher).await?;
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

async fn base_request<Source, Transport, Fetcher, Blocks, Ids, Lookup>(
    model_id: &str,
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
    let optional_params = filtered_params(model_id, request, sources).await?;
    let prepared = prepare(model_id, request, optional_params, sources).await?;
    let mut body: Object = serde_json::from_value(serde_json::to_value(&prepared.body)?)?;
    apply_bedrock_rules(model_id, &mut body, sources).await;
    Ok(body)
}

// The parent's _build_bedrock_anthropic_request_base filters every credential field, clamps the effort ceiling and
// normalizes tool-search tool types before it builds anything.
async fn filtered_params<Source, Transport, Fetcher, Blocks, Ids, Lookup>(
    model_id: &str,
    request: &ClaudeInvokeRequest<'_>,
    sources: &TransformSources<'_, '_, Source, Transport, Fetcher, Blocks, Ids, Lookup>,
) -> Result<IndexMap<String, ParamJson>, ClaudeInvokeError<Blocks::Failure>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
    Lookup: ModelInfoLookup,
{
    let mut optional_params: IndexMap<String, ParamJson> = request
        .optional_params
        .iter()
        .filter(|(key, _value)| !AUTHENTICATION_PARAMS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    normalize_output_config_effort(
        model_id,
        &mut optional_params,
        sources.lookup,
        sources.packaged,
    )
    .await?;
    normalize_tool_search_tools(&mut optional_params)?;
    Ok(optional_params)
}

async fn prepare<Source, Transport, Fetcher, Blocks, Ids, Lookup>(
    model_id: &str,
    request: &ClaudeInvokeRequest<'_>,
    optional_params: IndexMap<String, ParamJson>,
    sources: &mut TransformSources<'_, '_, Source, Transport, Fetcher, Blocks, Ids, Lookup>,
) -> Result<PreparedRequest, ClaudeInvokeError<Blocks::Failure>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
    Lookup: ModelInfoLookup,
{
    let anthropic_request = AnthropicRequest {
        model: model_id,
        custom_llm_provider: Some(CUSTOM_LLM_PROVIDER),
        messages: request.messages.clone(),
        optional_params,
        litellm_params: request.litellm_params.clone(),
        headers: request.headers.clone(),
        defaults: request.defaults,
        catalog: request.catalog,
        default_chat_max_tokens: request.default_chat_max_tokens,
        billing_header: BillingHeader::from(STRIPS_BILLING_METADATA),
        modify_params: request.modify_params,
        global_drop_params: request.global_drop_params,
    };
    Ok(anthropic_request
        .prepare(&mut *sources.environment, sources.catalogs, sources.prompts)
        .await?)
}

// The Bedrock invoke rules the parent applies after the native preparation; the model and stream stay out of the
// body, since the Invoke surface puts them in the URL.
async fn apply_bedrock_rules<Source, Transport, Fetcher, Blocks, Ids, Lookup>(
    model_id: &str,
    body: &mut Object,
    sources: &mut TransformSources<'_, '_, Source, Transport, Fetcher, Blocks, Ids, Lookup>,
) where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
    Lookup: ModelInfoLookup,
{
    body.shift_remove(MODEL_FIELD);
    body.shift_remove(STREAM_FIELD);
    let mut capabilities = MantleSchemaCapabilities {
        environment: &mut *sources.environment,
        catalogs: sources.catalogs,
    };
    apply_structured_output(model_id, body, &mut capabilities).await;
    strip_unsupported_config_keys(model_id, body, &mut capabilities).await;
    if !body.contains_key(ANTHROPIC_VERSION_FIELD) {
        body.insert(
            ANTHROPIC_VERSION_FIELD.to_owned(),
            BodyJson::Text(ANTHROPIC_VERSION.to_owned()),
        );
    }
    normalize_tool_custom_field(body);
    normalize_tool_schema_types(body);
    if let Some(tools) = strip_eager_input_streaming(body) {
        let stripped: Vec<BodyJson> = tools.into_iter().map(BodyJson::Object).collect();
        body.insert(TOOLS_FIELD.to_owned(), BodyJson::List(stripped));
    }
}

// B690 clamps output_config.effort through the model's declared ceiling and leaves every other value type alone.
async fn normalize_output_config_effort<FileFailure, Lookup>(
    model: &str,
    optional_params: &mut IndexMap<String, ParamJson>,
    lookup: &Lookup,
    packaged: &ModelCatalog,
) -> Result<(), ClaudeInvokeError<FileFailure>>
where
    FileFailure: StdError + Send + Sync + 'static,
    Lookup: ModelInfoLookup,
{
    let Some(value) = optional_params
        .get(OUTPUT_CONFIG_FIELD)
        .filter(|value| matches!(value, ParamJson::Object(_)))
    else {
        return Ok(());
    };
    let mut output_config: ParamValue = serde_json::from_value(serde_json::to_value(value)?)?;
    normalize_opus_effort(model, &mut output_config, lookup, packaged).await?;
    let normalized: ParamJson = serde_json::from_value(serde_json::to_value(&output_config)?)?;
    optional_params.insert(OUTPUT_CONFIG_FIELD.to_owned(), normalized);
    Ok(())
}

// B597's schema helpers ask the capabilities through the Bedrock provider B2221 already passes.
struct MantleSchemaCapabilities<'context, 'environment, Source, Transport>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    environment: &'context mut CapabilityEnvironment<'environment, Source, Transport>,
    catalogs: &'context CatalogSources<'environment>,
}

impl<Source, Transport> InvokeSchemaCapabilities
    for MantleSchemaCapabilities<'_, '_, Source, Transport>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    async fn supports_factory(&mut self, request: &CapabilityRequest<'_>, key: &str) -> bool {
        self.environment.supports_factory(request, key).await
    }

    async fn supports_effort_param(&mut self, model: &str) -> bool {
        model_supports_effort_param(
            model,
            CUSTOM_LLM_PROVIDER,
            &mut *self.environment,
            self.catalogs,
        )
        .await
    }
}
