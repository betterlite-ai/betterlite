use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use betterlite_bedrock::passthrough::sdk_call::{BedrockSdkCall, NoCredentialChain};
use betterlite_config::loader::ConfigError;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::passthrough_call::aws::AwsPassthrough;
use betterlite_core::api::passthrough_call::{
    PassthroughCallArgs, PassthroughCallError, PassthroughReply, allm_passthrough_route,
};
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::Utc;
use indexmap::IndexMap;
use serde::Deserialize;
use uuid::Uuid;

use super::convert::{
    API_ERROR, BAD_REQUEST, BAD_REQUEST_STATUS, ConvertError, ErrorPayload, SYNTHESIZED_STATUS,
    parse,
};

mod tests;

const CALL_TYPE: &str = "passthrough";
const MODEL_KEY: &str = "model";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SessionWire {
    // absent means the process environment, the snapshot the proxy policy is resolved from
    environment: Option<BTreeMap<String, String>>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error("the proxy environment could not be read: {source}")]
    Proxy {
        #[source]
        source: EnvError,
    },
    #[error("the outbound http client could not be built: {source}")]
    Client {
        #[source]
        source: HttpError,
    },
    #[error("the AWS pass-through settings could not be read: {source}")]
    Settings {
        #[source]
        source: ConfigError,
    },
}

// Everything one Python passthrough call owns: the outbound client every provider request goes through, the environment its provider settings read and the AWS port built from it
pub(crate) struct SessionCore {
    client: ProxiedClient,
    environment: BTreeMap<String, String>,
    aws: Arc<dyn AwsPassthrough>,
}

impl SessionCore {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = parse(text)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment)
            .map_err(|source| SessionError::Proxy { source })?;
        let client = ProxiedClient::outbound(&Arc::new(policy))
            .map_err(|source| SessionError::Client { source })?;
        let aws =
            BedrockSdkCall::from_environment(&environment, NoCredentialChain, SystemTime::now)
                .map_err(|source| SessionError::Settings { source })?;
        Ok(Self {
            client,
            environment,
            aws: Arc::new(aws),
        })
    }

    pub(crate) async fn call(
        &self,
        args: PassthroughCallArgs,
    ) -> Result<PassthroughReply, Box<ErrorPayload>> {
        let model = args.model.clone();
        let provider = args.custom_llm_provider.clone().unwrap_or_default();
        let mut logging = logging_state(&model)?;
        allm_passthrough_route(
            args,
            &mut logging,
            &self.client,
            &self.environment,
            &self.aws,
        )
        .await
        .map_err(|error| Box::new(failure(&error, &provider, &model)))
    }
}

// llm_passthrough_route's except block: the provider's own failure class and its fields
fn failure(error: &PassthroughCallError, provider: &str, model: &str) -> ErrorPayload {
    match error {
        PassthroughCallError::UpstreamStatus {
            status,
            headers,
            body,
        } => ErrorPayload::upstream(*status, headers, body),
        PassthroughCallError::ProviderNotFound(name) => ErrorPayload::provider_not_found(name),
        PassthroughCallError::Resolution(reason) => {
            ErrorPayload::failed(BAD_REQUEST, Some(BAD_REQUEST_STATUS), reason.to_string())
        }
        PassthroughCallError::MantleEndpoint(source) => {
            ErrorPayload::failed(BAD_REQUEST, Some(BAD_REQUEST_STATUS), source.to_string())
        }
        PassthroughCallError::Mapped(mapped) => ErrorPayload::provider(mapped, provider, model),
        PassthroughCallError::Transport(transport) => {
            ErrorPayload::failed(API_ERROR, Some(SYNTHESIZED_STATUS), transport.to_string())
        }
        PassthroughCallError::Routing { status, message } => {
            ErrorPayload::routing(*status, message)
        }
    }
}

// the logging state the proxy hands the route in kwargs, built here from the call alone
pub(crate) fn logging_state(model: &str) -> Result<LoggingState, Box<ErrorPayload>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| {
        Box::new(ErrorPayload::failed(
            API_ERROR,
            Some(SYNTHESIZED_STATUS),
            error.to_string(),
        ))
    })?;
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let origin = Instant::now();
    let clock = move || origin.elapsed();
    LoggingState::new(
        logging_init(model),
        setup,
        &redactor,
        || Uuid::new_v4().to_string(),
        clock,
    )
    .map_err(|error| {
        Box::new(ErrorPayload::failed(
            API_ERROR,
            Some(SYNTHESIZED_STATUS),
            error.to_string(),
        ))
    })
}

fn logging_init(model: &str) -> LoggingInit {
    LoggingInit {
        model: model.to_owned(),
        call_type: CALL_TYPE.to_owned(),
        start_time: Utc::now().fixed_offset(),
        litellm_call_id: Uuid::new_v4().to_string(),
        function_id: String::new(),
        messages: None,
        stream: None,
        litellm_trace_id: None,
        kwargs: IndexMap::from([(MODEL_KEY.to_owned(), Json::Text(model.to_owned()))]),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    }
}
