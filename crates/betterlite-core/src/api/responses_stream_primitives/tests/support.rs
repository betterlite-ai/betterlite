use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::api_base_lookup::{
    ApiBaseError, ApiBaseHooks, ApiBaseParams, LoginFreeProvider,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use crate::api::responses_stream_primitives::state::{
    LoggingSnapshot, ResponsesApiBase, StreamTimes,
};

pub(super) const MOMENT: &str = "2026-01-01T00:00:00+00:00";

pub(super) fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(MOMENT).expect("the fixture is RFC 3339")
}

pub(super) fn times() -> StreamTimes {
    StreamTimes {
        now: moment(),
        created_timestamp: 0.0,
    }
}

pub(super) fn snapshot(litellm_params: IndexMap<String, Json>) -> LoggingSnapshot {
    LoggingSnapshot {
        start_time: Some(moment()),
        litellm_params,
    }
}

pub(super) struct NoApiBase;

impl ResponsesApiBase for NoApiBase {
    fn resolve(
        &self,
        _model: &str,
        _params: &ApiBaseParams,
    ) -> Result<Option<String>, ApiBaseError> {
        Ok(None)
    }
}

pub(super) struct FailingApiBase;

impl ResponsesApiBase for FailingApiBase {
    fn resolve(
        &self,
        _model: &str,
        _params: &ApiBaseParams,
    ) -> Result<Option<String>, ApiBaseError> {
        Err(ApiBaseError::InvalidVertexLocation {
            message: "vertex".to_owned(),
        })
    }
}

pub(super) struct NoHooks;

impl ApiBaseHooks for NoHooks {
    fn login_free_api_base(&self, _provider: LoginFreeProvider) -> Option<String> {
        None
    }

    fn vertex_claude_url(
        &self,
        _location: &str,
        _project: &str,
        _model: &str,
        _stream: bool,
    ) -> Result<String, ApiBaseError> {
        Err(ApiBaseError::InvalidVertexLocation {
            message: String::new(),
        })
    }
}

pub(super) struct SharedResolution {
    pub(super) catalog: ModelCatalog,
    pub(super) known: BTreeSet<String>,
    pub(super) custom: BTreeSet<String>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) aliases: BTreeMap<String, String>,
}

impl Default for SharedResolution {
    fn default() -> Self {
        Self {
            catalog: ModelCatalog::default(),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
            aliases: BTreeMap::new(),
        }
    }
}

pub(super) fn resolution_context(resolution: &SharedResolution) -> ResolutionContext<'_> {
    ResolutionContext {
        catalog: &resolution.catalog,
        known_model_names: &resolution.known,
        custom_providers: &resolution.custom,
        environment: &resolution.environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) fn logging_object(
    start_time: DateTime<FixedOffset>,
    litellm_params: IndexMap<String, Json>,
) -> StreamLoggingObject {
    let mut context = CorrelationContext::default();
    let logging = LoggingState::new(
        LoggingInit {
            model: "gpt-4o".to_owned(),
            call_type: "responses".to_owned(),
            start_time,
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: Some(true),
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params,
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds");
    StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: context,
        })),
        dispatch: Arc::new(NoDispatch),
    }
}
