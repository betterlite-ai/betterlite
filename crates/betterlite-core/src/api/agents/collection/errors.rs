use std::io::{Error as IoError, ErrorKind};

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext, TaxonomyError};
use betterlite_http::interactions::agents_create_read::AgentRequestError;
use betterlite_http::interactions::base::{ClientError, RequestFailure};
use betterlite_redact::secret::Redactor;

use super::{AgentCollectionError, AgentCollectionFailure, AgentFailure, AgentPreparationError};

pub(super) struct AgentCallIdentity {
    model: String,
    provider: String,
    // HTTPHandler.post turns a transport timeout into litellm.Timeout; HTTPHandler.get lets it reach the client as a connection error
    post: bool,
}

pub(super) fn identity_of(model: &str, provider: &str, post: bool) -> AgentCallIdentity {
    AgentCallIdentity {
        model: model.to_owned(),
        provider: provider.to_owned(),
        post,
    }
}

pub(super) fn failure<ProviderError>(
    error: AgentCollectionError<ProviderError>,
    identity: &AgentCallIdentity,
    redactor: &Redactor,
) -> AgentCollectionFailure<ProviderError>
where
    ProviderError: AgentFailure,
{
    AgentCollectionFailure {
        class: class_of(&error, identity, redactor),
        error,
    }
}

fn class_of<ProviderError>(
    error: &AgentCollectionError<ProviderError>,
    identity: &AgentCallIdentity,
    redactor: &Redactor,
) -> ErrorClass
where
    ProviderError: AgentFailure,
{
    match error {
        AgentCollectionError::Prepare(AgentPreparationError::Config(configuration)) => {
            configuration.class()
        }
        AgentCollectionError::Prepare(AgentPreparationError::FieldType { field }) => {
            field_class(identity, field)
        }
        AgentCollectionError::Prepare(AgentPreparationError::Logging(source)) => {
            raw_class(identity, &source.to_string(), None, redactor)
        }
        AgentCollectionError::Request(AgentRequestError::Provider(source)) => {
            raw_class(identity, &source.message(), source.status_code(), redactor)
        }
        AgentCollectionError::Request(
            AgentRequestError::Client(ClientError::FieldType { field, .. })
            | AgentRequestError::Parameters(ClientError::FieldType { field, .. }),
        ) => field_class(identity, field),
        AgentCollectionError::Request(
            AgentRequestError::Client(source) | AgentRequestError::Parameters(source),
        ) => raw_class(identity, &source.to_string(), None, redactor),
        AgentCollectionError::Request(AgentRequestError::Transport(RequestFailure::Transport(
            source,
        ))) if identity.post && source.is_timeout() => TaxonomyError::Timeout {
            provider: identity.provider.clone(),
            model: Some(identity.model.clone()),
            cause: IoError::new(ErrorKind::TimedOut, source.to_string()),
        }
        .classify(),
        AgentCollectionError::Request(AgentRequestError::Transport(source)) => {
            raw_class(identity, &source.to_string(), None, redactor)
        }
    }
}

// docs/compatibility.md rejects a value outside the declared type with a 400 naming the field
fn field_class(identity: &AgentCallIdentity, field: &'static str) -> ErrorClass {
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
        ErrorContext {
            message: format!("the {field} value must be a string"),
            model: Some(identity.model.clone()),
            provider: Some(identity.provider.clone()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: Some(field.to_owned()),
            body_code: None,
        },
        None,
    )))
}

fn raw_class(
    identity: &AgentCallIdentity,
    text: &str,
    status_code: Option<u16>,
    redactor: &Redactor,
) -> ErrorClass {
    dispatch(
        &DispatchInput::Raw(Box::new(RawException {
            model: identity.model.clone(),
            custom_llm_provider: identity.provider.clone(),
            error_str: text.to_owned(),
            status_code,
            status_code_is_synthesized: false,
            message: Some(text.to_owned()),
            body: None,
            exception_type: String::new(),
        })),
        redactor,
    )
}
