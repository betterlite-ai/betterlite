mod tests;

use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::error::ProviderError;
use crate::openai_base_config::{Headers, Object};
use crate::passthrough::transformation::types::{
    PassthroughError, PassthroughLogging, PassthroughTarget, PassthroughUrlRequest, RelayedBody,
    RelayedResponse,
};
use crate::passthrough::transformation::{
    BasePassthroughConfig, model_group_from, relayed_body, strip_leading_model_segment,
};

const AUTHORIZATION: &str = "Authorization";
const STREAM: &str = "stream";

#[derive(Debug, Error)]
pub enum NvidiaNimPassthroughError {
    #[error(
        "NVIDIA NIM api base not found: set `api_base` on the deployment or NVIDIA_NIM_API_BASE"
    )]
    MissingApiBase,
    #[error(transparent)]
    Passthrough(#[from] PassthroughError),
    #[error(transparent)]
    Provider(Box<ProviderError>),
}

impl From<ProviderError> for NvidiaNimPassthroughError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(Box::new(error))
    }
}

// the caller resolves NVIDIA_NIM_API_BASE and NVIDIA_NIM_API_KEY, and an empty value counts as unset
#[derive(Debug, Clone, Default)]
pub struct NvidiaNimPassthroughConfig {
    pub api_base_env: Option<String>,
    pub api_key_env: Option<SecretString>,
}

impl NvidiaNimPassthroughConfig {
    #[must_use]
    pub fn get_api_base<'base>(&'base self, api_base: Option<&'base str>) -> Option<&'base str> {
        api_base
            .filter(|base| !base.is_empty())
            .or(self.api_base_env.as_deref().filter(|base| !base.is_empty()))
    }

    #[must_use]
    pub fn get_api_key<'key>(
        &'key self,
        api_key: Option<&'key SecretString>,
    ) -> Option<&'key SecretString> {
        api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or(self
                .api_key_env
                .as_ref()
                .filter(|key| !key.expose_secret().is_empty()))
    }
}

#[must_use]
pub fn validate_environment(headers: Headers, api_key: Option<&SecretString>) -> Headers {
    let Some(api_key) = api_key else {
        return headers;
    };
    let mut merged = headers;
    merged.insert(
        AUTHORIZATION.to_owned(),
        format!("Bearer {}", api_key.expose_secret()),
    );
    merged
}

pub fn without_repeated_version_prefix(
    api_base: &str,
    native_endpoint: &str,
) -> Result<String, NvidiaNimPassthroughError> {
    let url = Url::parse(api_base).map_err(|source| PassthroughError::InvalidUrl {
        url: api_base.to_owned(),
        source,
    })?;
    let base_segments: Vec<&str> = url
        .path_segments()
        .map(|segments| segments.filter(|segment| !segment.is_empty()).collect())
        .unwrap_or_default();
    let first_native_segment = native_endpoint
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("");
    let repeated = base_segments
        .last()
        .is_some_and(|last| *last == first_native_segment && is_version_segment(first_native_segment));
    let kept = if repeated {
        base_segments
            .get(..base_segments.len().saturating_sub(1))
            .unwrap_or_default()
    } else {
        base_segments.as_slice()
    };
    let mut rebuilt = url;
    rebuilt.set_path(&format!("/{}", kept.join("/")));
    rebuilt.set_query(None);
    Ok(rebuilt.as_str().trim_end_matches('/').to_owned())
}

fn is_version_segment(segment: &str) -> bool {
    segment
        .strip_prefix('v')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

impl BasePassthroughConfig for NvidiaNimPassthroughConfig {
    type Error = NvidiaNimPassthroughError;
    type Logged = RelayedBody;

    fn is_streaming_request(&self, _endpoint: &str, request_data: &Object) -> bool {
        matches!(request_data.get(STREAM), Some(Value::Bool(true)))
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        let Some(base) = self.get_api_base(request.api_base) else {
            return Err(NvidiaNimPassthroughError::MissingApiBase);
        };
        let names = [
            request.target.model.to_owned(),
            model_group_from(request.litellm_params),
        ];
        let native_endpoint = strip_leading_model_segment(request.target.endpoint, &names);
        let root = without_repeated_version_prefix(base, &native_endpoint)?;
        let url = self.format_url(&native_endpoint, &root, request.request_query_params)?;
        Ok((url, root))
    }

    fn logging_non_streaming_response(
        &self,
        _target: &PassthroughTarget<'_>,
        response: &RelayedResponse<'_>,
        _request_data: &Object,
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Self::Logged> {
        relayed_body(response.body).ok()
    }
}
