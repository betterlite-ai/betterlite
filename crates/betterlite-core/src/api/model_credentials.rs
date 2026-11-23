use std::convert::Infallible;
use std::error::Error;

use betterlite_error::proxy_model_not_found::ProxyModelNotFoundError;
use betterlite_models::controls::call_params::{TRUSTED_CREDENTIALS_KEY, TrustedCredentials};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use secrecy::SecretString;
use thiserror::Error;

mod tests;

// LiteLLM's default operation_context; Rust has no default arguments, so each caller passes this one
pub const DEFAULT_OPERATION_CONTEXT: &str = "file operation";

// The detail the router check answers with; it is an HTTPException and has no error type of its own
pub const ROUTER_MISSING_DETAIL: &str = "Router not initialized. Cannot use model-based routing.";

// Router.get_deployment_credentials_with_provider, answered by whichever router the server hands the file and batch routes
pub trait CredentialSource {
    type Error: Error + Send + Sync + 'static;

    fn credentials_with_provider(
        &self,
        model_id: &str,
        team_id: Option<&str>,
    ) -> Result<Option<JsonObject>, Self::Error>;
}

// the three failures of get_credentials_for_model: two HTTPException branches and the port's own error
#[derive(Debug, Error)]
pub enum CredentialLookupError<SourceError = Infallible>
where
    SourceError: Error + 'static,
{
    #[error("{}", ROUTER_MISSING_DETAIL)]
    RouterMissing,
    #[error(transparent)]
    ModelNotFound(#[from] ProxyModelNotFoundError),
    #[error("the credential source failed to resolve model `{model_id}`")]
    Source {
        model_id: String,
        #[source]
        source: SourceError,
    },
}

impl<SourceError> CredentialLookupError<SourceError>
where
    SourceError: Error + 'static,
{
    // only the two HTTPException branches carry a status; a port failure keeps whatever the router answered with
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::RouterMissing => Some(500),
            Self::ModelNotFound(_) => Some(ProxyModelNotFoundError::STATUS.as_u16()),
            Self::Source { .. } => None,
        }
    }

    // HTTPException's detail is {"error": <text>} for both branches LiteLLM raises itself
    #[must_use]
    pub fn detail_error(&self) -> Option<&str> {
        match self {
            Self::RouterMissing => Some(ROUTER_MISSING_DETAIL),
            Self::ModelNotFound(error) => Some(&error.detail.error),
            Self::Source { .. } => None,
        }
    }
}

// get_credentials_for_model: the router check first, then the lookup, then the missing-model error with retry false
pub fn get_credentials_for_model<Source>(
    source: Option<&Source>,
    model_id: &str,
    operation_context: &str,
) -> Result<JsonObject, CredentialLookupError<Source::Error>>
where
    Source: CredentialSource,
{
    let Some(router) = source else {
        return Err(CredentialLookupError::RouterMissing);
    };
    let found = router
        .credentials_with_provider(model_id, None)
        .map_err(|failure| CredentialLookupError::Source {
            model_id: model_id.to_owned(),
            source: failure,
        })?;
    found.ok_or_else(|| {
        CredentialLookupError::ModelNotFound(ProxyModelNotFoundError::new(
            operation_context,
            model_id,
            false,
        ))
    })
}

// prepare_data_with_credentials: dict.update, the snapshot, custom_llm_provider leaving data, then the file id
pub fn prepare_data_with_credentials(
    data: &mut JsonObject,
    credentials: &JsonObject,
    file_id: Option<&str>,
    include_internal_credentials: bool,
) -> Option<TrustedCredentials> {
    for (name, value) in credentials {
        data.insert(name.clone(), value.clone());
    }
    let snapshot = include_internal_credentials.then(|| trusted_snapshot(credentials));
    if include_internal_credentials {
        // LiteLLM assigns its read-only mapping over whatever the caller's data carried under this key
        data.shift_remove(TRUSTED_CREDENTIALS_KEY);
    }
    data.shift_remove("custom_llm_provider");
    if let Some(id) = file_id {
        data.insert("file_id".to_owned(), Json::Text(id.to_owned()));
    }
    snapshot
}

// the snapshot's readers take text fields only, so objects and lists such as vertex_credentials and the tag lists stay out
#[must_use]
pub fn trusted_snapshot(credentials: &JsonObject) -> TrustedCredentials {
    let values = credentials
        .iter()
        .filter_map(|(name, value)| match value {
            Json::Text(text) => Some((name.clone(), SecretString::from(text.clone()))),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect();
    TrustedCredentials { values }
}
