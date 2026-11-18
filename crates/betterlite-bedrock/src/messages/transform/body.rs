use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::capabilities::effort_param::model_supports_effort_param;
use betterlite_anthropic::feature_flags::FeatureParams;
use betterlite_anthropic::params::output_config::DROP_OUTPUT_CONFIG_WARNING;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::{
    CapabilityEnvironment, CapabilityPredicates as _, CapabilityRequest,
};
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::{Json, Object};

use super::{
    ANTHROPIC_VERSION, BEDROCK_ANTHROPIC_API_VERSION, MessagesRequest, MessagesTransformError,
    TransformSources,
};
use crate::bedrock_messages_beta::beta::{BetaHeadersRequest, beta_headers};
use crate::bedrock_messages_beta::fields::strip_unsupported_fields;
use crate::bedrock_messages_beta::provider::CUSTOM_LLM_PROVIDER;
use crate::bedrock_messages_beta::web_search::reject_web_search_tools;
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::bedrock_model_capabilities::effort::normalize_opus_effort;
use crate::invoke_schema::{
    InvokeSchemaCapabilities, apply_structured_output, ensure_tool_names,
    normalize_tool_custom_field, normalize_tool_schema_types, strip_eager_input_streaming,
    strip_unsupported_config_keys,
};
use crate::message_options::OptionsSources;
use crate::message_options::cache_ttl::remove_ttl_from_cache_control;
use crate::message_options::thinking::ensure_clear_thinking;
use crate::model_routes::{ParamMap, ParamValue};

const ANTHROPIC_BETA: &str = "anthropic_beta";
const COMPACTION: &str = "compaction";
const CONTAINER: &str = "container";
const EFFORT: &str = "effort";
const FORMAT: &str = "format";
const MCP_SERVERS: &str = "mcp_servers";
const MODEL: &str = "model";
const OUTPUT_CONFIG: &str = "output_config";
const REASONING_EFFORT: &str = "reasoning_effort";
const SAFEGUARDS: &str = "safeguards";
const STREAM: &str = "stream";
const TOOLS: &str = "tools";

// B597's schema helpers ask the capabilities through the Bedrock provider B2221 already passes.
struct SchemaCapabilities<'context, 'environment, Source, Transport>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    environment: &'context mut CapabilityEnvironment<'environment, Source, Transport>,
    catalogs: &'context CatalogSources<'environment>,
}

impl<Source, Transport> InvokeSchemaCapabilities for SchemaCapabilities<'_, '_, Source, Transport>
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

// The Bedrock invoke rules LiteLLM applies after its native preparation, in its order.
pub(super) async fn finish<Source, Transport, Lookup>(
    request: &MessagesRequest<'_>,
    sources: &mut TransformSources<'_, '_, Source, Transport, Lookup>,
    mut body: Object,
) -> Result<Object, MessagesTransformError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    reject_web_search_tools(&body, request.model)?;
    insert_version(&mut body);
    body.shift_remove(STREAM);
    body.shift_remove(MODEL);
    let injected = prepare_cache(&mut body, request, sources).await?;
    let params = feature_params(&body);
    apply_invoke_rules(request, sources, &mut body).await?;
    apply_betas(request, sources, &params, injected, &mut body).await?;
    drop_unsupported_output_config(
        &mut body,
        request,
        &mut *sources.environment,
        sources.catalogs,
    )
    .await;
    Ok(strip_unsupported_fields(&body)?)
}

// DEFAULT_BEDROCK_ANTHROPIC_API_VERSION fills the version only when the request names none.
fn insert_version(body: &mut Object) {
    if body.contains_key(ANTHROPIC_VERSION) {
        return;
    }
    body.insert(
        ANTHROPIC_VERSION.to_owned(),
        Json::Text(BEDROCK_ANTHROPIC_API_VERSION.to_owned()),
    );
}

async fn prepare_cache<Source, Transport, Lookup>(
    body: &mut Object,
    request: &MessagesRequest<'_>,
    sources: &mut TransformSources<'_, '_, Source, Transport, Lookup>,
) -> Result<bool, MessagesTransformError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let mut params: ParamMap = serde_json::from_value(serde_json::to_value(&*body)?)?;
    let injected = ensure_clear_thinking(
        &mut params,
        request.model,
        &mut OptionsSources {
            environment: &mut *sources.environment,
            catalogs: sources.catalogs,
            budgets: sources.budgets,
        },
    )
    .await?;
    remove_ttl_from_cache_control(&mut params, Some(request.model), request.model_cost)?;
    *body = serde_json::from_value(serde_json::to_value(&params)?)?;
    Ok(injected)
}

// LiteLLM shallow-copies output_config before B597 rewrites it so the beta detection reads the
// shape the native preparation left; Rust owns this object, so the copy is the snapshot taken
// before apply_structured_output runs.
fn feature_params(body: &Object) -> FeatureParams {
    FeatureParams {
        compaction: body.get(COMPACTION).map(ContractJson::from),
        container: body.get(CONTAINER).map(ContractJson::from),
        output_config: body.get(OUTPUT_CONFIG).map(ContractJson::from),
        reasoning_effort: body.get(REASONING_EFFORT).map(ContractJson::from),
    }
}

async fn apply_invoke_rules<Source, Transport, Lookup>(
    request: &MessagesRequest<'_>,
    sources: &mut TransformSources<'_, '_, Source, Transport, Lookup>,
    body: &mut Object,
) -> Result<(), MessagesTransformError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let mut capabilities = SchemaCapabilities {
        environment: &mut *sources.environment,
        catalogs: sources.catalogs,
    };
    apply_structured_output(request.model, body, &mut capabilities).await;
    normalize_effort(request.model, body, sources.lookup, sources.packaged).await?;
    strip_unsupported_config_keys(request.model, body, &mut capabilities).await;
    normalize_tool_custom_field(body);
    normalize_tool_schema_types(body);
    ensure_tool_names(body);
    Ok(())
}

async fn apply_betas<Source, Transport, Lookup>(
    request: &MessagesRequest<'_>,
    sources: &mut TransformSources<'_, '_, Source, Transport, Lookup>,
    params: &FeatureParams,
    injected: bool,
    body: &mut Object,
) -> Result<(), MessagesTransformError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let tools = optional_list(&request.optional_params, TOOLS);
    let mcp_servers = optional_list(&request.optional_params, MCP_SERVERS);
    let betas = beta_headers(
        BetaHeadersRequest {
            model: request.model,
            messages: &request.messages,
            tools: &tools,
            mcp_servers: &mcp_servers,
            safeguards: request.optional_params.get(SAFEGUARDS),
            params,
            headers: request.headers,
            outgoing: body,
            clear_thinking_injected: injected,
            subclass_provider: None,
        },
        &mut *sources.environment,
        sources.catalogs,
        sources.matrix,
    )
    .await?;
    insert_betas(body, betas);
    strip_eager_tools(body);
    Ok(())
}

fn optional_list(optional_params: &Object, key: &str) -> Vec<Json> {
    match optional_params.get(key) {
        Some(Json::List(items)) => items.clone(),
        _ => Vec::new(),
    }
}

fn insert_betas(body: &mut Object, betas: Vec<String>) {
    if betas.is_empty() {
        return;
    }
    body.insert(
        ANTHROPIC_BETA.to_owned(),
        Json::List(betas.into_iter().map(Json::Text).collect()),
    );
}

fn strip_eager_tools(body: &mut Object) {
    let Some(tools) = strip_eager_input_streaming(body) else {
        return;
    };
    body.insert(
        TOOLS.to_owned(),
        Json::List(tools.into_iter().map(Json::Object).collect()),
    );
}

// LiteLLM's final output_config drop. Its own pipeline never reaches it, since the strip above
// removes every non-format key exactly when the effort lookup says no, but the contract stays.
pub(crate) async fn drop_unsupported_output_config<Source, Transport>(
    body: &mut Object,
    request: &MessagesRequest<'_>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let other_keys = matches!(
        body.get(OUTPUT_CONFIG),
        Some(Json::Object(fields)) if fields.keys().any(|key| key != FORMAT)
    );
    if !request.drop_params || !other_keys {
        return;
    }
    if model_supports_effort_param(request.model, CUSTOM_LLM_PROVIDER, environment, catalogs).await
    {
        return;
    }
    tracing::warn!(
        "{}",
        DROP_OUTPUT_CONFIG_WARNING.replacen("%s", request.model, 1)
    );
    body.shift_remove(OUTPUT_CONFIG);
}

// B690 rewrites output_config.effort in place and leaves every other value type alone.
async fn normalize_effort<Lookup>(
    model: &str,
    body: &mut Object,
    lookup: &Lookup,
    packaged: &ModelCatalog,
) -> Result<(), MessagesTransformError>
where
    Lookup: ModelInfoLookup,
{
    let Some(Json::Object(fields)) = body.get(OUTPUT_CONFIG) else {
        return Ok(());
    };
    if !matches!(fields.get(EFFORT), Some(Json::Text(_))) {
        return Ok(());
    }
    let declared = Json::Object(fields.clone());
    let mut normalized: ParamValue = serde_json::from_value(serde_json::to_value(declared)?)?;
    normalize_opus_effort(model, &mut normalized, lookup, packaged).await?;
    let rebuilt = serde_json::from_value(serde_json::to_value(&normalized)?)?;
    body.insert(OUTPUT_CONFIG.to_owned(), rebuilt);
    Ok(())
}
