mod tests;

use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::error::ProviderError;
use crate::openai_base_config::Object;
use crate::passthrough::transformation::BasePassthroughConfig;
use crate::passthrough::transformation::types::{
    PassthroughError, PassthroughUrlRequest, RelayedBody,
};
use crate::watsonx_auth::credentials::get_watsonx_credentials;
use crate::watsonx_auth::environment::WatsonxEnvironment;
use crate::watsonx_auth::error::WatsonxError;
use crate::watsonx_auth::types::WatsonxParams;
use crate::watsonx_auth::urls::get_base_url;

const STREAM: &str = "stream";

#[derive(Debug, Error)]
pub enum WatsonxPassthroughError {
    #[error(transparent)]
    Watsonx(Box<WatsonxError>),
    #[error(transparent)]
    Passthrough(#[from] PassthroughError),
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
}

impl From<ProviderError> for WatsonxPassthroughError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(Box::new(error))
    }
}

impl From<WatsonxError> for WatsonxPassthroughError {
    fn from(error: WatsonxError) -> Self {
        Self::Watsonx(Box::new(error))
    }
}

impl WatsonxPassthroughError {
    // the passthrough relay keeps the shared credential status and leaves every other failure untyped
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Watsonx(error) => match error.as_ref() {
                WatsonxError::Api { status_code, .. } => Some(*status_code),
                WatsonxError::IamApiKeyRequired
                | WatsonxError::InvalidField { .. }
                | WatsonxError::InvalidHeader { .. }
                | WatsonxError::IamUrl(_)
                | WatsonxError::IamStatus { .. }
                | WatsonxError::IamTransport(_)
                | WatsonxError::IamReply(_)
                | WatsonxError::IamReplyField { .. }
                | WatsonxError::Cache(_) => None,
            },
            Self::Passthrough(_) | Self::Provider(_) => None,
        }
    }
}

pub struct WatsonxPassthroughConfig<'env> {
    pub environment: &'env WatsonxEnvironment,
}

impl WatsonxPassthroughConfig<'_> {
    // LiteLLM reaches the credentials helper even when only a key is needed, so a missing base still refuses
    pub fn get_api_key(
        &self,
        api_key: Option<&SecretString>,
    ) -> Result<SecretString, WatsonxPassthroughError> {
        if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
            return Ok(key.clone());
        }
        let credentials =
            get_watsonx_credentials(&mut WatsonxParams::new(), None, None, self.environment)?;
        Ok(credentials.api_key)
    }
}

impl BasePassthroughConfig for WatsonxPassthroughConfig<'_> {
    type Error = WatsonxPassthroughError;
    type Logged = RelayedBody;

    fn is_streaming_request(&self, _endpoint: &str, request_data: &Object) -> bool {
        matches!(request_data.get(STREAM), Some(Value::Bool(true)))
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        let base = get_base_url(request.api_base, self.environment)?;
        let url = self.format_url(request.target.endpoint, &base, request.request_query_params)?;
        Ok((url, base))
    }
}
