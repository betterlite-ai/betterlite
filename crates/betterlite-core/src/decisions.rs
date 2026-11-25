use std::sync::Mutex;
use std::time::Duration;

use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::openai::DecisionResponse;
use betterlite_models::decisions::systemone::DecisionsResponse as SystemOneDecisionsResponse;
use betterlite_models::decisions::{Extra, Json};
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::decisions::call::azure::AzureAiConfig;
use betterlite_openai::decisions::call::error::PrepareError;
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;
use thiserror::Error;

use crate::api::decisions_send::clients::{DecisionsAsyncTransport, DecisionsSyncTransport};
use crate::api::decisions_send::error::DecisionsSendError;
use crate::api::decisions_send::format::DecisionsOutput;
use crate::api::decisions_send::send::DecisionsSender;

mod adecisions;
pub(crate) mod decisions_entrypoint;

#[expect(clippy::pub_use, reason = "litellm.decisions exports adecisions")]
pub use adecisions::adecisions;
pub type OpenAiDecisionResponse = DecisionResponse;
pub type SystemOneResponse = SystemOneDecisionsResponse;

use decisions_entrypoint::prepare_decisions_call;

// litellm.decisions' arguments, with each request format's values still as the caller sent them
pub struct DecisionsArgs {
    pub model: String,
    pub input: Option<Json>,
    pub questions: Option<Json>,
    pub safety_identifier: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<String>,
    pub extra_headers: Option<Headers>,
    pub kwargs: Extra,
}

// the request-building values B3513's prepare_call needs, which the caller owns and hands in per call
pub struct DecisionsPrepare<'deps> {
    pub resolution: &'deps ResolutionContext<'deps>,
    pub globals: LiteLlmGlobals,
    pub azure_ai: AzureAiConfig,
    pub logging: Option<&'deps Mutex<LoggingState>>,
    pub global_drop_params: bool,
}

pub struct DecisionsDeps<'deps, Clients> {
    pub clients: &'deps mut Clients,
    pub env: &'deps PrePostEnv<'deps>,
    pub prepare: DecisionsPrepare<'deps>,
}

#[derive(Debug, Error)]
pub enum DecisionsEntryError {
    #[error(transparent)]
    Prepare(#[from] PrepareError),
    #[error(transparent)]
    Send(#[from] DecisionsSendError),
    #[error("Invalid Decisions request: {message}")]
    Fields { message: String },
    #[error("the systemone call returned the {format} response format")]
    ResponseFormat { format: &'static str },
}

impl DecisionsEntryError {
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::Prepare(error) => error.class(),
            Self::Send(DecisionsSendError::Mapped { class, .. }) => class.clone(),
            Self::Send(DecisionsSendError::Format(error)) => {
                ErrorClass::InternalServer(Box::new(error_context(error.to_string())))
            }
            Self::Fields { message } => ErrorClass::BadRequest(Box::new(ResponseDetails::from(
                error_context(message.clone()),
            ))),
            Self::ResponseFormat { .. } => {
                ErrorClass::InternalServer(Box::new(error_context(self.to_string())))
            }
        }
    }
}

const fn error_context(message: String) -> ErrorContext {
    ErrorContext {
        message,
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

// litellm.decisions: _prepare_decisions_call, then B3514's blocking send
pub fn decisions<Clients>(
    deps: DecisionsDeps<'_, Clients>,
    args: DecisionsArgs,
) -> Result<DecisionsOutput, DecisionsEntryError>
where
    Clients: DecisionsSyncTransport + DecisionsAsyncTransport,
{
    let call = prepare_decisions_call(deps.prepare, args)?;
    let mut sender = DecisionsSender::new(deps.clients, deps.env);
    sender.send(&call).map_err(DecisionsEntryError::from)
}
