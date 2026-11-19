use std::collections::BTreeSet;

use betterlite_anthropic::anthropic_capability_catalog::thinking::is_effort_used;
use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, OptionalParams};
use betterlite_anthropic::feature_flags::beta::user_beta_headers;
use betterlite_anthropic::feature_flags::messages::{
    is_cache_control_set, is_file_id_used, is_mcp_server_used, is_pdf_used,
};
use betterlite_anthropic::feature_flags::tools::{
    computer_tool_version, is_code_execution_tool_used, is_container_with_skills_used,
    is_tool_search_used, is_web_search_tool_used, uses_input_examples, uses_programmatic_calls,
};
use betterlite_anthropic::feature_flags::{FeatureFlagsError, FeatureParams};
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::headers::request::{
    HeaderFeature, HeaderFeatures, HeaderRequest, get_anthropic_headers,
};
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::request::error::RequestError;
use betterlite_anthropic::request::{AnthropicRequest, BillingHeader, PreparedRequest};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::claude_platform_inputs::body::PlatformWarning;
use crate::model_routes::ParamMap;

use super::convert;
use super::error::ChatValidationError;
use super::{
    CUSTOM_LLM_PROVIDER, MESSAGES_PROVIDER, STRIPS_BILLING_METADATA, WORKSPACE_HEADER,
    WorkspaceSources, require_workspace, resolve_api_key,
};

pub const BETA_HEADER: &str = "anthropic-beta";

pub struct ChatHeadersRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub optional_params: &'request IndexMap<String, Json>,
    pub caller_headers: &'request IndexMap<String, String>,
    pub api_key: Option<&'request SecretString>,
    pub workspace: WorkspaceSources<'request>,
}

pub struct HeaderSources<'context, 'environment, Source, Transport> {
    pub environment: &'context mut CapabilityEnvironment<'environment, Source, Transport>,
    pub catalogs: &'context CatalogSources<'environment>,
}

// BedrockClaudePlatformConfig.validate_environment: the caller's headers with the generated ones laid over them.
pub async fn validate_environment<Source, Transport>(
    request: &ChatHeadersRequest<'_>,
    sources: &mut HeaderSources<'_, '_, Source, Transport>,
) -> Result<AnthropicHeaders, ChatValidationError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let contract: ParamMap = convert::mapped(request.optional_params)?;
    let workspace = require_workspace(request.model, &contract, request.workspace)?;
    let messages = convert::prompt_messages(request.messages)?;
    let params: IndexMap<String, PromptJson> = convert::mapped(request.optional_params)?;
    let effort_used = is_effort_used(
        Some(&optional_params_view(request.optional_params)),
        request.model,
        MESSAGES_PROVIDER,
        &mut *sources.environment,
        sources.catalogs,
    )
    .await;
    let features = header_features(&messages, &params, request.optional_params, effort_used)?;
    let api_key = resolve_api_key(request.api_key, request.workspace.environment);
    let user_betas = user_beta_headers(request.caller_headers.get(BETA_HEADER).map(String::as_str));
    let mut generated = get_anthropic_headers(&HeaderRequest {
        api_key: api_key.as_ref(),
        auth_token: None,
        anthropic_version: None,
        api_base: None,
        use_bearer_for_custom_base: false,
        user_betas: user_betas.as_deref(),
        features: &features,
    });
    generated
        .values
        .insert(WORKSPACE_HEADER.to_owned(), workspace);
    let mut merged = request.caller_headers.clone();
    merged.extend(generated.values);
    Ok(AnthropicHeaders { values: merged })
}

// Python reads a JSON null and an absent key the same way, so both reads filter the null out.
fn optional_params_view(params: &IndexMap<String, Json>) -> OptionalParams {
    OptionalParams {
        reasoning_effort: params
            .get("reasoning_effort")
            .filter(|value| !matches!(value, Json::Null))
            .cloned(),
        thinking: params
            .get("thinking")
            .filter(|value| !matches!(value, Json::Null))
            .cloned(),
        output_config: params
            .get("output_config")
            .filter(|value| !matches!(value, Json::Null))
            .cloned(),
    }
}

fn list_param<'params>(
    params: &'params IndexMap<String, PromptJson>,
    key: &str,
) -> Option<&'params [PromptJson]> {
    match params.get(key) {
        Some(PromptJson::List(items)) => Some(items.as_slice()),
        None | Some(_) => None,
    }
}

fn header_features(
    messages: &[PromptJson],
    params: &IndexMap<String, PromptJson>,
    contract: &IndexMap<String, Json>,
    effort_used: bool,
) -> Result<HeaderFeatures, FeatureFlagsError> {
    let tools = list_param(params, "tools").unwrap_or_default();
    let mut features = BTreeSet::new();
    let detected = [
        (HeaderFeature::PromptCaching, is_cache_control_set(messages)),
        (HeaderFeature::Pdf, is_pdf_used(messages)),
        (HeaderFeature::FileId, is_file_id_used(messages)?),
        (
            HeaderFeature::McpServer,
            is_mcp_server_used(list_param(params, "mcp_servers")),
        ),
        (HeaderFeature::ToolSearch, is_tool_search_used(tools)),
        (
            HeaderFeature::ProgrammaticToolCalling,
            uses_programmatic_calls(tools),
        ),
        (HeaderFeature::InputExamples, uses_input_examples(tools)),
        (
            HeaderFeature::CodeExecution,
            is_code_execution_tool_used(tools),
        ),
        (
            HeaderFeature::Skills,
            is_container_with_skills_used(&FeatureParams {
                compaction: None,
                container: contract
                    .get("container")
                    .filter(|value| !matches!(value, Json::Null))
                    .cloned(),
                output_config: None,
                reasoning_effort: None,
            }),
        ),
        (HeaderFeature::Effort, effort_used),
    ];
    for (feature, used) in detected {
        if used {
            features.insert(feature);
        }
    }
    Ok(HeaderFeatures {
        features,
        computer_tool_used: tools.iter().find_map(computer_tool_version),
        is_vertex_request: false,
        web_search_tool_used: is_web_search_tool_used(tools),
    })
}

pub struct ChatRequest<'request> {
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

pub struct ChatSources<'context, 'environment, Source, Transport, Fetcher, Blocks, Ids>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    pub environment: &'context mut CapabilityEnvironment<'environment, Source, Transport>,
    pub catalogs: &'context CatalogSources<'environment>,
    pub prompts: &'context PromptDeps<'context, Fetcher, Blocks, Ids>,
}

pub struct ChatPrepared {
    pub prepared: PreparedRequest,
    pub warnings: Vec<PlatformWarning>,
}

// transform_request filters through B2211 and hands the rest to the inherited Anthropic chat mapper.
pub async fn transform_request<Source, Transport, Fetcher, Blocks, Ids>(
    request: &ChatRequest<'_>,
    sources: &mut ChatSources<'_, '_, Source, Transport, Fetcher, Blocks, Ids>,
) -> Result<ChatPrepared, RequestError<Blocks::Failure>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
{
    let (filtered, warnings) = convert::filtered_params(
        &request.optional_params,
        &request.litellm_params,
        true,
        true,
    )?;
    let anthropic_request = AnthropicRequest {
        model: request.model,
        custom_llm_provider: Some(CUSTOM_LLM_PROVIDER),
        messages: request.messages.clone(),
        optional_params: convert::mapped(&filtered)?,
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
        .prepare(&mut *sources.environment, sources.catalogs, sources.prompts)
        .await?;
    Ok(ChatPrepared { prepared, warnings })
}
