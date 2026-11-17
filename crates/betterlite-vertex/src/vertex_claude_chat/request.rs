use std::collections::BTreeSet;
use std::error::Error as StdError;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::request::beta_headers::ANTHROPIC_BETA_HEADER;
use betterlite_anthropic::request::error::RequestError;
use betterlite_anthropic::request::{AnthropicRequest, BillingHeader, PreparedRequest};
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::media::inline::{RemoteMediaFetcher, async_inline_remote_media};
use betterlite_prompts::media::parts::INLINE_REMOTE_IMAGE_URLS;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::PROVIDER;
use super::beta::compose;
use super::error::ClaudeChatError;
use super::output::sanitize_output_params;

const MODEL: &str = "model";
const EXTRA_HEADERS: &str = "extra_headers";
const ANTHROPIC_BETA: &str = "anthropic_beta";
const VERTEX_REQUEST: &str = "is_vertex_request";

// Inline every image_url, drop model and extra_headers, and write the betas to the body list and the header.
pub async fn prepare<Source, Transport, Fetcher, Blocks, Ids>(
    mut request: AnthropicRequest<'_>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
    prompts: &PromptDeps<'_, Fetcher, Blocks, Ids>,
) -> Result<PreparedRequest, ClaudeChatError<Blocks::Failure>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
{
    let typed: Vec<Message> = convert_each(&request.messages)?;
    let inlined = async_inline_remote_media(&typed, &INLINE_REMOTE_IMAGE_URLS, prompts.fetcher)
        .await
        .map_err(RequestError::Media)?;
    if inlined != typed {
        request.messages = convert_each(&inlined)?;
    }
    request.custom_llm_provider = Some(PROVIDER);
    request.billing_header = BillingHeader::Strip;
    request
        .optional_params
        .insert(VERTEX_REQUEST.to_owned(), Json::Bool(true));
    let mut prepared = request.prepare(environment, catalogs, prompts).await?;
    let betas = compose(
        request.model,
        &prepared.body,
        &request.messages,
        environment,
        catalogs,
    )
    .await?;
    prepared.body.shift_remove(MODEL);
    prepared.body.shift_remove(EXTRA_HEADERS);
    sanitize_output_params(&mut prepared.body, request.model, environment, catalogs).await;
    if !betas.is_empty() {
        write_betas(&mut prepared.body, &mut prepared.headers, &betas);
    }
    Ok(prepared)
}

// One JSON string pass converts between the message and wire records, keeping arbitrary-precision numbers out.
fn convert_each<Source, Target, FileFailure>(
    items: &[Source],
) -> Result<Vec<Target>, RequestError<FileFailure>>
where
    Source: Serialize,
    Target: DeserializeOwned,
    FileFailure: StdError + Send + Sync + 'static,
{
    let mut converted = Vec::with_capacity(items.len());
    for item in items {
        let encoded = serde_json::to_string(item).map_err(RequestError::Convert)?;
        converted.push(serde_json::from_str(&encoded).map_err(RequestError::Convert)?);
    }
    Ok(converted)
}

// LiteLLM writes set order; betterlite sorts so the body list and the comma-joined header are run-stable.
fn write_betas(
    body: &mut IndexMap<String, Json>,
    headers: &mut AnthropicHeaders,
    betas: &BTreeSet<String>,
) {
    let listed: Vec<Json> = betas.iter().map(|beta| Json::Text(beta.clone())).collect();
    body.insert(ANTHROPIC_BETA.to_owned(), Json::List(listed));
    let joined = betas
        .iter()
        .map(String::as_str)
        .collect::<Vec<&str>>()
        .join(",");
    headers
        .values
        .insert(ANTHROPIC_BETA_HEADER.to_owned(), joined);
}
