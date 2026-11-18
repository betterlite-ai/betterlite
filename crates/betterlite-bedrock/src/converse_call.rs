use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_http::user_url::validate::Resolve;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
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
use futures_util::FutureExt as _;
use futures_util::future::LocalBoxFuture;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::agentcore_stream::IdSource;
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::converse_request::{ConverseMedia, ConverseSettings};
use crate::converse_response::wire::ModelResponse;
use crate::image_call::plan::LogContext;
use crate::invoke_events::DecoderClock;
use crate::model_routes::ParamMap;
use crate::rerank::plan::CredentialSource;
use crate::rerank::transport::{ClientKind, SuppliedClient};

pub mod buffered;
pub mod error;
mod prepare;
pub mod request;
pub mod streaming;
pub mod target;
pub mod transport;

mod tests;

use self::buffered::{BufferedParts, BufferedReply, BufferedTail, send_buffered};
use self::error::ConverseCallError;
use self::prepare::{CallPlan, ExtractInput};
use self::streaming::{StreamParts, WrappedStream, open_stream};
use self::transport::{ConverseClientFactory, ConverseSigner, SuppliedStream};

#[derive(Clone, Copy, Default)]
pub struct ConverseEnvironment<'text> {
    pub region_name: Option<&'text str>,
    pub region: Option<&'text str>,
    pub session_region: Option<&'text str>,
    pub runtime_endpoint: Option<&'text str>,
    pub bearer_token: Option<&'text SecretString>,
}

// The option maps stay owned so their pops, the stored region and the internal marker are visible to
// the caller once the call returns.
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

// Everything a call reaches outside itself: the credential source, the signer, the clients, the beta
// matrix, the clock the conversion reads and the logging hooks.
pub struct ConverseDeps<'deps, Factory, Cred>
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
    pub redactor: &'deps Redactor,
    pub betas: &'deps BetaMatrix,
    pub now: SystemTime,
    pub state: &'deps mut LoggingState,
    pub env: &'deps PrePostEnv<'deps>,
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
    Reply(BufferedReply),
    Stream(Box<WrappedStream<'deps>>),
}

// completion() with both routes; a local future because the logging env is not Sync.
pub fn converse_call<'run, Cred, Source, Transport, Factory, Lookup, R, Ids>(
    call: &'run mut ConverseCall<'run>,
    deps: ConverseDeps<'run, Factory, Cred>,
    assembly: ConverseAssembly<'run, 'run, Source, Transport, Lookup, R, Ids>,
) -> LocalBoxFuture<'run, Result<ConverseOutput<'run>, ConverseCallError>>
where
    Cred: CredentialSource + Sync,
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Factory: ConverseClientFactory,
    Lookup: ModelInfoLookup,
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    async move {
        let prepared = prepare::extract(
            ExtractInput {
                params: &mut call.optional_params,
                model: call.model,
                extra_headers: call.extra_headers,
                control: call.control,
                api_key: call.api_key,
                environment: call.environment,
            },
            deps.betas,
        )?;
        let plan =
            prepare::prepare(call, prepared, deps.signer, deps.credentials, assembly).await?;
        dispatch(&*call, plan, deps).await
    }
    .boxed_local()
}

// async_completion alone honours skip_pre_call_logging; the streaming and sync routes always log.
async fn dispatch<'run, Factory, Cred>(
    call: &ConverseCall<'_>,
    plan: CallPlan,
    mut deps: ConverseDeps<'run, Factory, Cred>,
) -> Result<ConverseOutput<'run>, ConverseCallError>
where
    Factory: ConverseClientFactory,
{
    let route = if call.acompletion {
        ClientKind::Async
    } else {
        ClientKind::Sync
    };
    let mut log = LogContext {
        state: &mut *deps.state,
        env: deps.env,
    };
    if !(call.acompletion && plan.stream != Some(true) && call.skip_pre_call_logging) {
        log_pre_call(&plan, &mut log)?;
    }
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
        let stream = open_stream(call, &plan, &parts, deps.factory, &mut log).await?;
        return Ok(ConverseOutput::Stream(Box::new(stream)));
    }
    let parts = BufferedParts {
        route,
        timeout: call.timeout,
        supplied: deps.supplied_buffered.as_ref(),
    };
    let reply = send_buffered(
        call,
        &plan,
        &parts,
        deps.factory,
        BufferedTail {
            now: deps.now,
            ids: deps.ids.as_ref(),
            log,
        },
    )
    .await?;
    Ok(ConverseOutput::Reply(reply))
}

// pre_call gets api_key="" and the signed headers, body and api_base the provider receives.
fn log_pre_call(plan: &CallPlan, log: &mut LogContext<'_, '_>) -> Result<(), ConverseCallError> {
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
