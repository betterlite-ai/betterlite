pub mod budget;
pub mod cache;
pub mod error;
pub mod fields;
pub mod logging;
pub mod max_tokens;
pub mod metadata;
pub mod ports;
pub mod post_call;
pub mod retry;
pub mod state;
pub mod stream;
pub mod target;

mod order_async;
mod order_sync;
mod raw_request;
mod steps;

mod tests;

use std::sync::{Arc, Mutex};

use betterlite_core::rules::RuleEvaluator;
use betterlite_core::schema::SchemaLimits;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::logging_state::correlation::{CorrelationContext, CorrelationSettings};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use self::cache::CacheHandler;
use self::error::CallError;
use self::logging::CallLogging;
use self::metadata::ResponseMetadataUpdater;
use self::ports::{BlockingBridge, IdSource, WrapperClock};
use self::retry::WrapperRetryDefaults;
use self::state::{KnownCallback, LoggingHandle, WrapperOptions};
use self::stream::StreamAssembly;
use self::target::CallTarget;
use crate::call_setup::RequestMap;
use crate::call_setup::callbacks::{CallbackLists, CustomLoggers};
use crate::call_setup::correlation::CallOutcome;
use crate::call_setup::credentials::CredentialLookup;
use crate::call_setup::function_setup::{
    BreadcrumbSink, CallSetup, CallSetupDependencies, TraceSource, function_setup,
};
use crate::call_setup::hooks::DeploymentHook;
use crate::call_setup::providers::ProviderRegistry;
use crate::call_setup::request_flags::is_async_request;

// What function_setup reads besides the request: module-level state in LiteLLM, injected here
pub struct SetupState {
    pub lists: CallbackLists,
    pub providers: ProviderRegistry,
    pub callbacks: Vec<Arc<dyn DeploymentHook>>,
    pub known: Arc<dyn KnownCallback>,
    pub resolver: Arc<dyn LoggerResolver>,
    pub correlation_settings: CorrelationSettings,
    pub redactor: Redactor,
    pub litellm_params: IndexMap<String, Json>,
    pub turn_off_message_logging: bool,
    pub breadcrumbs: Option<Arc<dyn BreadcrumbSink>>,
}

// litellm.modify_params and litellm.enable_json_schema_validation plus B200's validation bounds
#[derive(Debug, Clone, Copy)]
pub struct WrapperFlags {
    pub modify_params: bool,
    pub enable_json_schema_validation: bool,
    pub schema_limits: SchemaLimits,
}

// Everything the client decorator read from litellm module state, plus the ports that replace its imports
pub struct CallWrapper<E> {
    pub rules: E,
    pub setup: SetupState,
    pub cache: Option<Arc<dyn CacheHandler>>,
    pub budget: budget::BudgetState,
    pub retry: WrapperRetryDefaults,
    pub clock: Arc<dyn WrapperClock>,
    pub ids: Arc<dyn IdSource>,
    pub max_tokens: Arc<dyn max_tokens::MaxTokensModifier>,
    pub stream_assembly: Arc<dyn StreamAssembly>,
    pub metadata: Arc<dyn ResponseMetadataUpdater>,
    pub credentials: Arc<dyn CredentialLookup>,
    pub logging: Arc<dyn CallLogging>,
    pub scheduler: Arc<dyn logging::SuccessLogScheduler>,
    pub debugging: bool,
    pub flags: WrapperFlags,
}

impl<E> CallWrapper<E>
where
    E: RuleEvaluator + Send + Sync,
{
    // function_setup with the caller's copies of the lists, the providers and the correlation context
    pub(crate) fn start_logging(
        &mut self,
        call_type: &str,
        request: RequestMap,
        is_async_call: bool,
    ) -> Result<(LoggingHandle, RequestMap), CallError> {
        let mut correlation = CorrelationContext::default();
        let setup = begin_setup(self, call_type, request, is_async_call, &mut correlation)?;
        let handle = LoggingHandle {
            state: Arc::new(Mutex::new(setup.logging)),
            correlation: Arc::new(Mutex::new(correlation)),
            deferred: Arc::new(Mutex::new(None)),
        };
        Ok((handle, setup.request))
    }

    // wrapper_async: the async order, with correlation ids and the background success log
    pub async fn call(
        &mut self,
        target: &dyn CallTarget,
        request: RequestMap,
        options: WrapperOptions,
    ) -> Result<CallOutcome, CallError> {
        order_async::AsyncRun::new(self, target, request, options)
            .run()
            .await
    }

    // wrapper: the sync order, which never stamps correlation ids and logs failures on the calling thread
    pub fn call_blocking<B>(
        &mut self,
        target: &dyn CallTarget,
        request: RequestMap,
        options: WrapperOptions,
        bridge: &B,
    ) -> Result<CallOutcome, CallError>
    where
        B: BlockingBridge,
    {
        if is_async_request(&request, false) {
            let logging = options.logging.as_ref();
            return order_sync::async_request_path(self, target, bridge, &request, logging);
        }
        order_sync::SyncRun::new(self, target, request, options, bridge).run()
    }

    // return_raw_request: the target runs with a raw request logging object whose key is never the caller's
    pub async fn return_raw_request(
        &self,
        target: &dyn CallTarget,
        mut request: RequestMap,
    ) -> Json {
        request.insert(
            "api_key".to_owned(),
            Json::Text(raw_request::RAW_REQUEST_FAKE_KEY.to_owned()),
        );
        let logging = raw_request::raw_logging(self, &request);
        raw_request::capture(logging, target, request).await
    }
}

// the dependency bundle function_setup takes, built from the setup state and the run's own context
fn begin_setup<E>(
    wrapper: &mut CallWrapper<E>,
    call_type: &str,
    request: RequestMap,
    is_async_call: bool,
    correlation: &mut CorrelationContext,
) -> Result<CallSetup, CallError>
where
    E: RuleEvaluator,
{
    let mut clock = {
        let source = Arc::clone(&wrapper.clock);
        move || source.monotonic()
    };
    let next_trace_id = {
        let source = Arc::clone(&wrapper.ids);
        move || source.next_call_id()
    };
    let known = Arc::clone(&wrapper.setup.known);
    let resolver = Arc::clone(&wrapper.setup.resolver);
    let known_fn = |name: &str| known.is_known(name);
    let breadcrumbs = wrapper
        .setup
        .breadcrumbs
        .as_ref()
        .map(|sink| -> &dyn BreadcrumbSink { sink.as_ref() });
    let deps = CallSetupDependencies {
        lists: &mut wrapper.setup.lists,
        providers: &mut wrapper.setup.providers,
        callbacks: &wrapper.setup.callbacks,
        loggers: CustomLoggers {
            known: &known_fn,
            resolver: resolver.as_ref(),
        },
        correlation,
        correlation_settings: wrapper.setup.correlation_settings,
        redactor: &wrapper.setup.redactor,
        trace: TraceSource {
            next_trace_id: &next_trace_id,
            clock: &mut clock,
        },
        breadcrumbs,
        litellm_params: wrapper.setup.litellm_params.clone(),
        turn_off_message_logging: wrapper.setup.turn_off_message_logging,
        is_async_call,
    };
    let start = wrapper.clock.now();
    function_setup(call_type, &wrapper.rules, start, request, deps).map_err(CallError::from)
}
