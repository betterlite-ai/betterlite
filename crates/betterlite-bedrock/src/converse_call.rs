use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_http::user_url::validate::Resolve;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::agentcore_stream::IdSource;
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::converse_request::{ConverseMedia, ConverseSettings};
use crate::converse_response::wire::ModelResponse;
use crate::credentials::CredentialSource;
use crate::image_call::plan::LogContext;
use crate::invoke_events::DecoderClock;
use crate::model_routes::ParamMap;
use crate::rerank::transport::RerankHttpReply;

pub mod buffered;
pub mod error;
pub mod prepare;
pub mod request;
pub mod streaming;
pub mod target;
pub mod transport;

mod tests;

use self::buffered::{BufferedParts, BufferedReply, BufferedTail, finish_buffered, send_buffered};
use self::error::ConverseCallError;
use self::prepare::{CallPlan, ExtractInput};
use self::streaming::{PendingStream, StreamParts, WrappedStream, finish_stream, open_stream};
use self::transport::{ConverseClientFactory, ConverseSigner, SuppliedStream};

#[derive(Clone, Copy, Default)]
pub struct ConverseEnvironment<'text> {
    pub region_name: Option<&'text str>,
    pub region: Option<&'text str>,
    pub session_region: Option<&'text str>,
    pub runtime_endpoint: Option<&'text str>,
    pub bearer_token: Option<&'text SecretString>,
}

// The option maps stay owned so their pops, the stored region and the internal marker are visible to the caller once the call returns.
pub struct ConverseCall<'text> {
    pub model: &'text str,
    pub messages: &'text [Message],
    pub optional_params: ParamMap,
    pub litellm_params: Object,
    pub extra_headers: Option<&'text [(String, String)]>,
    pub api_base: Option<&'text str>,
    pub api_key: Option<&'text SecretString>,
    pub timeout: Option<Duration>,
    pub acompletion: bool,
    pub control: ControlOptions,
    pub skip_pre_call_logging: bool,
    pub model_response: ModelResponse,
    pub settings: ConverseSettings<'text>,
    pub environment: ConverseEnvironment<'text>,
    pub now: SystemTime,
}

// The auth parameters and the api key are credentials, so Debug names neither their keys nor values.
impl Debug for ConverseCall<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let header_names: Vec<&str> = self
            .extra_headers
            .unwrap_or_default()
            .iter()
            .map(|(name, _value)| name.as_str())
            .collect();
        f.debug_struct("ConverseCall")
            .field("model", &self.model)
            .field("acompletion", &self.acompletion)
            .field("timeout", &self.timeout)
            .field("stream_chunk_size", &self.control.stream_chunk_size)
            .field("api_key_present", &self.api_key.is_some())
            .field(
                "bearer_token_present",
                &self.environment.bearer_token.is_some(),
            )
            .field("header_names", &header_names)
            .field("skip_pre_call_logging", &self.skip_pre_call_logging)
            .finish_non_exhaustive()
    }
}

// Send-safe dependencies shared by request preparation and transport. Logging hooks stay in the synchronous stages.
pub struct ConverseDeps<'deps, 'stream, Factory, Cred>
where
    Factory: ConverseClientFactory,
{
    pub credentials: &'deps Cred,
    pub signer: &'deps dyn ConverseSigner,
    pub factory: &'deps Factory,
    pub supplied_buffered: Option<SuppliedClient<'deps, Factory::Buffered>>,
    pub supplied_stream: Option<SuppliedStream>,
    pub ids: Arc<dyn IdSource>,
    pub clock: DecoderClock,
    pub redactor: &'stream Redactor,
    pub betas: &'deps BetaMatrix,
    pub now: SystemTime,
}

// The body and capability inputs a request transform reads; the credential source stays in deps.
pub struct ConverseAssembly<'assembly, 'environment, Source, Transport, Lookup, R, Ids> {
    pub environment: &'assembly mut CapabilityEnvironment<'environment, Source, Transport>,
    pub lookup: &'assembly Lookup,
    pub media: ConverseMedia<'assembly, R, Ids>,
    pub model_cost: &'assembly ParamMap,
    pub catalogs: &'assembly CatalogSources<'assembly>,
    pub packaged: &'assembly ModelCatalog,
}

pub enum ConverseOutput<'deps> {
    Reply(Box<BufferedReply>),
    Stream(Box<WrappedStream<'deps>>),
}

// Prepare once, then log_pre_call, await dispatch, and finish with the caller's logging hooks.
pub async fn converse_call<Cred, Source, Transport, Factory, Lookup, R, Ids>(
    call: &mut ConverseCall<'_>,
    deps: &ConverseDeps<'_, '_, Factory, Cred>,
    assembly: ConverseAssembly<'_, '_, Source, Transport, Lookup, R, Ids>,
) -> Result<CallPlan, ConverseCallError>
where
    Cred: CredentialSource,
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Factory: ConverseClientFactory,
    Lookup: ModelInfoLookup,
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let prepared = prepare::extract(
        &mut ExtractInput {
            params: &mut call.optional_params,
            model: call.model,
            extra_headers: call.extra_headers,
            control: call.control,
            api_key: call.api_key,
            environment: call.environment,
        },
        deps.betas,
    )?;
    prepare::prepare(call, prepared, deps.signer, deps.credentials, assembly).await
}

pub enum PendingCall<'stream> {
    Reply(RerankHttpReply),
    Stream(Box<PendingStream<'stream>>),
}

// Only transport state crosses the await; logging hooks are invoked before and after it.
pub async fn dispatch<'stream, Factory, Cred>(
    call: &ConverseCall<'_>,
    plan: &CallPlan,
    deps: &ConverseDeps<'_, 'stream, Factory, Cred>,
    state: &LoggingState,
) -> Result<PendingCall<'stream>, ConverseCallError>
where
    Factory: ConverseClientFactory,
    Cred: Sync,
{
    let route = if call.acompletion {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    };
    if plan.stream == Some(true) {
        let parts = StreamParts {
            route,
            timeout: call.timeout,
            supplied: deps.supplied_stream.as_ref(),
            ids: Arc::clone(&deps.ids),
            clock: deps.clock,
            redactor: deps.redactor,
            now: deps.now,
        };
        let pending = open_stream(call, plan, &parts, deps.factory).await?;
        return Ok(PendingCall::Stream(Box::new(pending)));
    }
    let parts = BufferedParts {
        route,
        timeout: call.timeout,
        supplied: deps.supplied_buffered.as_ref(),
    };
    Ok(PendingCall::Reply(
        send_buffered(deps.factory, &parts, plan, state).await?,
    ))
}

pub fn finish<'stream>(
    call: &ConverseCall<'_>,
    plan: &CallPlan,
    pending: PendingCall<'stream>,
    mut tail: BufferedTail<'_, '_>,
) -> Result<ConverseOutput<'stream>, ConverseCallError> {
    match pending {
        PendingCall::Reply(reply) => Ok(ConverseOutput::Reply(Box::new(finish_buffered(
            call, plan, &reply, tail,
        )?))),
        PendingCall::Stream(stream) => Ok(ConverseOutput::Stream(Box::new(finish_stream(
            call,
            plan,
            *stream,
            &mut tail.log,
        )?))),
    }
}

// pre_call gets api_key="" and the signed headers, body and api_base the provider receives.
pub fn log_pre_call(
    call: &ConverseCall<'_>,
    plan: &CallPlan,
    log: &mut LogContext<'_, '_>,
) -> Result<(), ConverseCallError> {
    if call.acompletion && plan.stream != Some(true) && call.skip_pre_call_logging {
        return Ok(());
    }
    let headers: IndexMap<String, Json> = plan
        .signed
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    pre_call(
        log.state,
        &PreCall {
            input: plan.messages.clone(),
            model: None,
            additional_args: AdditionalArgs {
                api_base: Some(plan.url.clone()),
                headers: Some(headers),
                complete_input_dict: Some(RawBody::Text(plan.data.clone())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    )?;
    Ok(())
}
