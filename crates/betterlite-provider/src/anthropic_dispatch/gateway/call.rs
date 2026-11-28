use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_http_stream::error::StreamFailure;
use betterlite_anthropic::chat_call::ChatOutcome;
use betterlite_anthropic::request::BillingHeader;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::stream_state::state::{IdSource, UuidV4};
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_core::cost::completion::lookup::NoHfConfig;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::validate::SystemResolver;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::resolution::ResolutionContext;
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::anthropic_invokes::file_blocks::RemoteFileBlocks;
use betterlite_prompts::media_fetch::fetcher::Fetcher;
use betterlite_tokenizer::engine::Engine;
use chrono::Utc;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::anthropic_dispatch::base::Surface;
use crate::anthropic_dispatch::completion;
use crate::anthropic_dispatch::completion::{CompletionCall, CompletionSettings};
use crate::anthropic_dispatch::gateway::context;
use crate::anthropic_dispatch::gateway::error;
use crate::anthropic_dispatch::gateway::files;
use crate::anthropic_dispatch::gateway::logging::{self, TracingSink};
use crate::anthropic_dispatch::gateway::parameters::{MappedParameters, Sources, map};
use crate::anthropic_dispatch::gateway::runtime::AnthropicRuntime;
use crate::anthropic_dispatch::gateway::{AnthropicUpstream, provider_headers};
use crate::anthropic_dispatch::native::legacy::Legacy;
use crate::anthropic_dispatch::native::preparation::Preparation;
use crate::anthropic_dispatch::native::{ClientHandle, Counter, Native, Reply, Runtime};
use crate::operation::OperationError;
use crate::prepared::body::chat_request_fields;
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};

type Media = Fetcher<SystemResolver>;
type Files<'fetch> = files::Files<'fetch, Media>;
type Blocks<'fetch> = RemoteFileBlocks<'fetch, Files<'fetch>>;
type Handler<'call> = Native<
    'call,
    ProxiedClient,
    NoDynamicMetadata,
    NoHfConfig,
    Files<'call>,
    Blocks<'call>,
    fn() -> String,
>;

// The request's messages and remaining fields, taken apart once so no held borrow ties the walk to them.
pub(crate) struct Request {
    pub(crate) messages: Vec<Json>,
    pub(crate) params: IndexMap<String, JsonValue>,
    pub(crate) stream: bool,
}

pub(crate) async fn execute(
    client: &ProxiedClient,
    prepared: &PreparedRequest,
    deployment: &Deployment,
) -> Result<Upstream, OperationError> {
    let PreparedRequest::Anthropic(inner) = prepared else {
        return Err(error::unsupported());
    };
    let runtime = deployment
        .anthropic
        .as_deref()
        .ok_or_else(error::unavailable)?;
    let fields = chat_request_fields(&inner.chat.request)?;
    let messages = serde_json::to_value(fields.messages)
        .and_then(serde_json::from_value)
        .map_err(|source| OperationError::Encode { source })?;
    let request = Request {
        messages,
        params: fields.params,
        stream: inner.stream,
    };
    let outcome = run(runtime, client, deployment, request).await?;
    let headers = provider_headers(&outcome);
    Ok(Upstream::Anthropic(Box::new(AnthropicUpstream {
        headers,
        outcome,
    })))
}

async fn run(
    runtime: &AnthropicRuntime,
    client: &ProxiedClient,
    deployment: &Deployment,
    request: Request,
) -> Result<ChatOutcome<StreamFailure, Counter>, OperationError> {
    let resolution = context::resolution(runtime);
    let mut keys = ModelCostKeys::default();
    let Mapped {
        capabilities,
        catalogs,
        parameters,
        messages,
        stream,
    } = mapped_request(runtime, &resolution, &mut keys, deployment, request).await?;
    let fetched = files::prefetch(&runtime.fetcher, &messages).await;
    let media = files::Files {
        fetcher: &runtime.fetcher,
        fetched: &fetched,
    };
    let blocks = RemoteFileBlocks { fetcher: &media };
    let matrix = runtime.beta.matrix().await;
    let mut native = Handler::from(Deps {
        runtime,
        deployment,
        client,
        matrix: &matrix,
        capabilities,
        catalogs,
        media: &media,
        blocks: &blocks,
        effort_used: parameters.effort_used,
    });
    let call = completion_call(runtime, deployment, &messages, stream, &parameters);
    send(&mut native, call, runtime, &deployment.model).await
}

// The borrowing handler structs are built from &runtime once per call, after the parameter mapping has released them.
struct Mapped<'call> {
    capabilities: CapabilityEnvironment<'call, NoDynamicMetadata, NoHfConfig>,
    catalogs: CatalogSources<'call>,
    parameters: MappedParameters,
    messages: Vec<Json>,
    stream: bool,
}

async fn mapped_request<'call>(
    runtime: &'call AnthropicRuntime,
    resolution: &'call ResolutionContext<'call>,
    keys: &'call mut ModelCostKeys,
    deployment: &'call Deployment,
    request: Request,
) -> Result<Mapped<'call>, OperationError> {
    let Request {
        messages,
        params,
        stream,
    } = request;
    let catalogs = context::catalogs(runtime);
    let mut capabilities = context::capability_environment(runtime, resolution, keys);
    let parameters = {
        let mut sources = Sources {
            runtime,
            deployment,
            environment: &mut capabilities,
            catalogs: &catalogs,
            model: &deployment.model,
        };
        map(&mut sources, params, stream).await?
    };
    Ok(Mapped {
        capabilities,
        catalogs,
        parameters,
        messages,
        stream,
    })
}

struct Deps<'call> {
    runtime: &'call AnthropicRuntime,
    deployment: &'call Deployment,
    client: &'call ProxiedClient,
    matrix: &'call BetaMatrix,
    capabilities: CapabilityEnvironment<'call, NoDynamicMetadata, NoHfConfig>,
    catalogs: CatalogSources<'call>,
    media: &'call Files<'call>,
    blocks: &'call Blocks<'call>,
    effort_used: bool,
}

impl<'call> From<Deps<'call>> for Handler<'call> {
    fn from(deps: Deps<'call>) -> Self {
        let ids: &dyn IdSource = &UuidV4;
        Self {
            runtime: Runtime {
                pooled: deps.client,
                settings: &deps.runtime.settings,
                matrix: deps.matrix,
                stream_logger: &deps.runtime.stream_logger,
                ids,
                now: Utc::now(),
                max_event_bytes: deps.runtime.max_event_bytes,
                max_error_body_bytes: deps.runtime.max_error_body_bytes,
                disable_add_prefix_to_prompt: deps
                    .runtime
                    .request_flags
                    .disable_add_prefix_to_prompt,
                effort_used: deps.effort_used,
            },
            preparation: Preparation {
                environment: deps.capabilities,
                catalogs: deps.catalogs,
                prompts: PromptDeps {
                    fetcher: deps.media,
                    blocks: deps.blocks,
                    new_id: &deps.runtime.new_id,
                },
                catalog: &deps.runtime.catalog,
                defaults: &deps.runtime.defaults,
                max_tokens: deps.runtime.max_tokens,
                billing: BillingHeader::Keep,
                modify_params: deps.runtime.request_flags.modify_params,
                drop_params: deps.deployment.global_drop_params,
            },
            legacy: Legacy {
                prompts: &deps.runtime.prompt_factory,
                defaults: &deps.runtime.legacy_defaults,
                body_limit: deps.runtime.max_error_body_bytes,
            },
        }
    }
}

async fn send(
    native: &mut Handler<'_>,
    call: CompletionCall<ClientHandle<ProxiedClient>, Engine>,
    runtime: &AnthropicRuntime,
    model: &str,
) -> Result<ChatOutcome<StreamFailure, Counter>, OperationError> {
    // The messages surface is always the provider's own config; selected_config needs it named.
    let settings = CompletionSettings {
        globals: &runtime.bases.globals,
        environment: &runtime.bases.environment,
        custom_prompt_dict: &runtime.custom_prompts,
        disable_suffix: runtime.bases.disable_suffix,
        selection: Some(ChatConfigTag::Anthropic),
    };
    let call_id = Uuid::new_v4().to_string();
    let (mut state, env) = logging::state(
        &TracingSink,
        &runtime.redactor,
        model,
        &call_id,
        Utc::now().fixed_offset(),
    )
    .map_err(|source| error::internal(source.to_string()))?;
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    match completion::complete(Surface::Messages, call, &settings, native, &mut log).await {
        Ok(Reply::Messages(outcome)) => Ok(outcome),
        Ok(Reply::Legacy(_)) => Err(error::unsupported()),
        Err(source) => Err(error::dispatch(source)),
    }
}

fn completion_call(
    runtime: &AnthropicRuntime,
    deployment: &Deployment,
    messages: &[Json],
    stream: bool,
    parameters: &MappedParameters,
) -> CompletionCall<ClientHandle<ProxiedClient>, Engine> {
    CompletionCall {
        model: deployment.model.clone(),
        provider: String::from("anthropic"),
        messages: messages.to_vec(),
        model_response: ResponseEnvelope::new(format!("chatcmpl-{}", Uuid::new_v4())),
        optional_params: parameters.optional.clone(),
        litellm_params: IndexMap::new(),
        headers: parameters
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        api_key: deployment.api_key.clone(),
        api_base: Some(deployment.api_base.clone()),
        custom_prompt_dict: BTreeMap::new(),
        stream,
        acompletion: true,
        client: None,
        shared_session: None,
        encoding: runtime.engine.clone(),
        timeout: deployment.timeout,
    }
}
