use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::request::{AnthropicRequest, BillingHeader};
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
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
use crate::invoke_schema::{
    apply_structured_output, normalize_tool_custom_field, normalize_tool_schema_types,
    strip_eager_input_streaming, strip_unsupported_config_keys,
};
use crate::model_routes::ParamValue;

use super::beta::{BETA_HEADER_FIELD, InvokeBetaRequest, compute_beta_headers};
use super::capabilities::InvokeCapabilities;
use super::documents::convert_document_url_sources;
use super::error::ClaudeInvokeError;
use super::tools::normalize_tool_search_tools;
use super::{ANTHROPIC_VERSION, CUSTOM_LLM_PROVIDER, STRIPS_BILLING_METADATA};

const MODELS: &str = "model";
const STREAM: &str = "stream";
const TOOLS: &str = "tools";

#[derive(Clone)]
pub struct ClaudeInvokeRequest<'request> {
    pub model: &'request str,
    pub messages: Vec<Json>,
    pub optional_params: IndexMap<String, Json>,
    pub litellm_params: IndexMap<String, Json>,
    pub headers: AnthropicHeaders,
    pub defaults: &'request OptionDefaults,
    pub catalog: &'request ModelCatalog,
    pub default_chat_max_tokens: i64,
    pub modify_params: bool,
    pub global_drop_params: bool,
}

pub struct TransformSources<'context, 'environment, Source, Transport, Fetcher, Blocks, Ids, Lookup>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    pub environment: &'context mut CapabilityEnvironment<'environment, Source, Transport>,
    pub catalogs: &'context CatalogSources<'environment>,
    pub prompts: &'context PromptDeps<'context, Fetcher, Blocks, Ids>,
    pub matrix: &'context BetaMatrix,
    pub lookup: &'context Lookup,
    pub packaged: &'context ModelCatalog,
}

pub struct ClaudeInvokePrepared {
    pub body: Object,
    pub tool_name_reverse: BTreeMap<String, String>,
}

pub(crate) struct BaseRequest {
    pub(crate) body: Object,
    pub(crate) headers: AnthropicHeaders,
    pub(crate) tool_name_reverse: BTreeMap<String, String>,
}

pub async fn transform_request<Source, Transport, Fetcher, Blocks, Ids, Lookup>(
    request: &ClaudeInvokeRequest<'_>,
    sources: &mut TransformSources<'_, '_, Source, Transport, Fetcher, Blocks, Ids, Lookup>,
) -> Result<ClaudeInvokePrepared, ClaudeInvokeError<Blocks::Failure>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
    Lookup: ModelInfoLookup,
{
    let base = sources.build_base_request(request).await?;
    let mut body = base.body;
    convert_document_url_sources(&mut body, sources.prompts.fetcher).await?;
    sources
        .apply_beta_headers(request, &base.headers, &mut body)
        .await?;
    Ok(ClaudeInvokePrepared {
        body,
        tool_name_reverse: base.tool_name_reverse,
    })
}

// LiteLLM inlines every remote part before the synchronous transform, so its failures come first.
pub async fn async_transform_request<Metadata, Client, Media, Files, Ids, Lookup>(
    request: &ClaudeInvokeRequest<'_>,
    sources: &mut TransformSources<'_, '_, Metadata, Client, Media, Files, Ids, Lookup>,
) -> Result<ClaudeInvokePrepared, ClaudeInvokeError<Files::Failure>>
where
    Metadata: DynamicMetadataSource,
    Client: ModelInfoClient,
    Media: RemoteMediaFetcher + Sync,
    Files: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
    Lookup: ModelInfoLookup,
{
    let inlined_request = ClaudeInvokeRequest {
        messages: inlined_messages::<Media, Files>(&request.messages, sources.prompts.fetcher)
            .await?,
        ..ClaudeInvokeRequest::clone(request)
    };
    transform_request(&inlined_request, sources).await
}

pub(crate) async fn inlined_messages<Fetcher, Blocks>(
    messages: &[Json],
    fetcher: &Fetcher,
) -> Result<Vec<Json>, ClaudeInvokeError<Blocks::Failure>>
where
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
{
    let typed: Vec<Message> = serde_json::from_value(serde_json::to_value(messages)?)?;
    let inlined = async_inline_remote_media(&typed, &INLINE_EVERY_REMOTE_URL, fetcher).await?;
    let converted: Vec<Json> = serde_json::from_value(serde_json::to_value(&inlined)?)?;
    Ok(converted)
}

impl<Source, Transport, Fetcher, Blocks, Ids, Lookup>
    TransformSources<'_, '_, Source, Transport, Fetcher, Blocks, Ids, Lookup>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
    Lookup: ModelInfoLookup,
{
    // _build_bedrock_anthropic_request_base: every step both the invoke and mantle chat transforms share, without the invoke surface's document conversion and computed beta list.
    pub(crate) async fn build_base_request(
        &mut self,
        request: &ClaudeInvokeRequest<'_>,
    ) -> Result<BaseRequest, ClaudeInvokeError<Blocks::Failure>> {
        let mut optional_params = authentication_filtered(&request.optional_params);
        self.normalize_output_config_effort(request.model, &mut optional_params)
            .await?;
        normalize_tool_search_tools(&mut optional_params)?;
        let anthropic_request = AnthropicRequest {
            model: request.model,
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
        let prepared = anthropic_request
            .prepare(&mut *self.environment, self.catalogs, self.prompts)
            .await?;
        let mut body: Object = serde_json::from_value(serde_json::to_value(&prepared.body)?)?;
        body.shift_remove(MODELS);
        body.shift_remove(STREAM);
        self.rewrite_shared_body(request, &mut body).await;
        Ok(BaseRequest {
            body,
            headers: prepared.headers,
            tool_name_reverse: prepared.tool_name_reverse,
        })
    }

    async fn rewrite_shared_body(&mut self, request: &ClaudeInvokeRequest<'_>, body: &mut Object) {
        self.apply_schema_rewrites(request, body).await;
        if !body.contains_key("anthropic_version") {
            body.insert(
                "anthropic_version".to_owned(),
                BodyJson::Text(ANTHROPIC_VERSION.to_owned()),
            );
        }
        normalize_tool_custom_field(body);
        normalize_tool_schema_types(body);
        strip_eager_streaming_tools(body);
    }

    // The schema helpers read both capability answers through the Bedrock provider.
    async fn apply_schema_rewrites(
        &mut self,
        request: &ClaudeInvokeRequest<'_>,
        body: &mut Object,
    ) {
        let mut capabilities = InvokeCapabilities {
            environment: &mut *self.environment,
            catalogs: self.catalogs,
        };
        apply_structured_output(request.model, body, &mut capabilities).await;
        strip_unsupported_config_keys(request.model, body, &mut capabilities).await;
    }

    // The parent transform merged its optional betas into these headers before it returned them.
    async fn apply_beta_headers(
        &mut self,
        request: &ClaudeInvokeRequest<'_>,
        headers: &AnthropicHeaders,
        body: &mut Object,
    ) -> Result<(), ClaudeInvokeError<Blocks::Failure>> {
        let betas = compute_beta_headers(
            &InvokeBetaRequest {
                model: request.model,
                messages: &request.messages,
                optional_params: &request.optional_params,
                headers,
                thinking: body.get("thinking"),
            },
            &mut *self.environment,
            self.catalogs,
            self.matrix,
        )
        .await?;
        if betas.is_empty() {
            return Ok(());
        }
        let list = BodyJson::List(betas.into_iter().map(BodyJson::Text).collect());
        body.insert(BETA_HEADER_FIELD.to_owned(), list);
        Ok(())
    }

    async fn normalize_output_config_effort(
        &self,
        model: &str,
        optional_params: &mut IndexMap<String, Json>,
    ) -> Result<(), ClaudeInvokeError<Blocks::Failure>> {
        let Some(value) = optional_params
            .get("output_config")
            .filter(|value| matches!(value, Json::Object(_)))
        else {
            return Ok(());
        };
        let mut output_config: ParamValue = serde_json::from_value(serde_json::to_value(value)?)?;
        normalize_opus_effort(model, &mut output_config, self.lookup, self.packaged).await?;
        let normalized: Json = serde_json::from_value(serde_json::to_value(&output_config)?)?;
        optional_params.insert("output_config".to_owned(), normalized);
        Ok(())
    }
}

fn strip_eager_streaming_tools(body: &mut Object) {
    let Some(tools) = strip_eager_input_streaming(body) else {
        return;
    };
    let stripped: Vec<BodyJson> = tools.into_iter().map(BodyJson::Object).collect();
    body.insert(TOOLS.to_owned(), BodyJson::List(stripped));
}

// Every AWS credential field LiteLLM's aws_authentication_params keeps out of the model body.
fn authentication_filtered(optional_params: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    optional_params
        .iter()
        .filter(|(key, _value)| !AUTHENTICATION_PARAMS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
