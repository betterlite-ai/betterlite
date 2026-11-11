use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_openai::gdc_policy::GdcPolicyError;
use thiserror::Error;

use crate::gdc_auth::cache::CacheError;
use crate::gdc_auth::credential::CredentialError;
use crate::gdc_auth::exchange::ExchangeError;
use crate::gdc_auth::key::KeyError;

const PROVIDER: &str = "gdc";

#[rustfmt::skip]
#[derive(Debug, Error)]
pub enum GdcAuthError {
    #[error("the GDC Gemini {what} is not set")] Missing { what: &'static str, model: String },
    #[error("the GDC Gemini API key is not a GDCH service account credential")] Key { model: String, #[source] source: KeyError },
    #[error("the GDC Gemini service account credential could not be loaded")] Credential { model: String, #[source] source: CredentialError },
    #[error("the GDC Gemini token could not be obtained")] Exchange { model: String, #[source] source: ExchangeError },
    #[error("the GDC Gemini token cache is not usable")] Cache { model: String, #[source] source: CacheError },
    #[error(transparent)] Policy(#[from] GdcPolicyError),
}

impl GdcAuthError {
    #[must_use]
    #[rustfmt::skip]
    pub fn class(&self) -> ErrorClass {
        let model = match self {
            Self::Missing { model, .. } | Self::Key { model, .. } | Self::Credential { model, .. } | Self::Exchange { model, .. } | Self::Cache { model, .. } => Some(model.clone()),
            Self::Policy(_) => None,
        };
        let context = ErrorContext { message: self.to_string(), model, provider: Some(PROVIDER.to_owned()), max_retries: None, num_retries: None, body_type: None, body_param: None, body_code: None };
        match self {
            Self::Missing { .. } | Self::Key { .. } | Self::Credential { .. } => ErrorClass::Authentication(Box::new(ResponseDetails::from(context))),
            Self::Exchange { .. } | Self::Cache { .. } => ErrorClass::ApiConnection(Box::new(context)),
            Self::Policy(error) => error.class(),
        }
    }
}
