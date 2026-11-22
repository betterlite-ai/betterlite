use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::resolution::ResolutionError;
use thiserror::Error;

// prepare_call's failures, each carrying the text and the llm_provider LiteLLM's exception carries
#[derive(Debug, Error)]
pub enum PrepareError {
    #[error("{source}")]
    Resolution {
        model: String,
        #[source]
        source: ResolutionError,
    },
    #[error("Unknown Decisions provider '{provider}'. Supported providers: {supported}")]
    UnknownProvider {
        model: String,
        provider: String,
        supported: String,
    },
    #[error("A model name is required for the Decisions API")]
    ModelRequired { model: String, provider: String },
    #[error("Invalid Decisions request: {source}")]
    InvalidRequest {
        model: String,
        provider: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("{message}")]
    MissingApiBase {
        message: String,
        model: String,
        provider: String,
    },
    #[error("Missing API key for Decisions provider '{provider}'")]
    MissingApiKey { model: String, provider: String },
    #[error(
        "{provider} does not support parameters: ['safety_identifier'], for model={model}. To drop these, set `litellm.drop_params=True` or for proxy:\n\n`litellm_settings:\n drop_params: true`\n"
    )]
    UnsupportedSafetyIdentifier { model: String, provider: String },
    #[error("Decisions provider '{provider}' cannot serve this request: {reason}")]
    UnsupportedRequest {
        model: String,
        provider: String,
        reason: String,
    },
    #[error("Requests with '{field}' go through litellm.{other_function}")]
    OtherFormat {
        model: String,
        field: String,
        other_function: String,
    },
    #[error("updating the decision call logging failed: {source}")]
    Logging {
        model: String,
        provider: String,
        #[source]
        source: LoggingStateError,
    },
}

impl PrepareError {
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::MissingApiKey { .. } => {
                ErrorClass::Authentication(Box::new(ResponseDetails::from(self.context())))
            }
            Self::UnsupportedSafetyIdentifier { .. } => {
                ErrorClass::UnsupportedParams(Box::new(self.context()))
            }
            Self::Logging { .. } => ErrorClass::InternalServer(Box::new(self.context())),
            Self::Resolution { .. }
            | Self::UnknownProvider { .. }
            | Self::ModelRequired { .. }
            | Self::InvalidRequest { .. }
            | Self::MissingApiBase { .. }
            | Self::UnsupportedRequest { .. }
            | Self::OtherFormat { .. } => {
                ErrorClass::BadRequest(Box::new(ResponseDetails::from(self.context())))
            }
        }
    }

    fn context(&self) -> ErrorContext {
        ErrorContext {
            message: self.to_string(),
            model: Some(self.model().to_owned()),
            provider: self.provider().map(str::to_owned),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        }
    }

    fn model(&self) -> &str {
        match self {
            Self::Resolution { model, .. }
            | Self::UnknownProvider { model, .. }
            | Self::ModelRequired { model, .. }
            | Self::InvalidRequest { model, .. }
            | Self::MissingApiBase { model, .. }
            | Self::MissingApiKey { model, .. }
            | Self::UnsupportedSafetyIdentifier { model, .. }
            | Self::UnsupportedRequest { model, .. }
            | Self::OtherFormat { model, .. }
            | Self::Logging { model, .. } => model,
        }
    }

    fn provider(&self) -> Option<&str> {
        match self {
            Self::Resolution { .. } => None,
            Self::OtherFormat { .. } => Some(""),
            Self::UnknownProvider { provider, .. }
            | Self::ModelRequired { provider, .. }
            | Self::InvalidRequest { provider, .. }
            | Self::MissingApiBase { provider, .. }
            | Self::MissingApiKey { provider, .. }
            | Self::UnsupportedSafetyIdentifier { provider, .. }
            | Self::UnsupportedRequest { provider, .. }
            | Self::Logging { provider, .. } => Some(provider),
        }
    }
}
