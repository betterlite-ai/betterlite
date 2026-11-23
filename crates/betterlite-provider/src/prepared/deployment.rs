use std::sync::Arc;
use std::time::{Duration, TryFromFloatSecsError};

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::controls::call_params::DropParams;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::headers::affinity::AffinityError;
use betterlite_openai::chat::deployment::{ChatDeployment, Provider as DeploymentProvider};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use url::Url;

use crate::anthropic_dispatch::gateway::runtime::AnthropicRuntime;
use crate::azure_dispatch::call::Composition;
use crate::operation::OperationError;
use crate::provider::Provider;

// LiteLLM's DEFAULT_REQUEST_TIMEOUT_SECONDS
const DEFAULT_TIMEOUT_SECONDS: f64 = 6000.0;
const DEFAULT_OPENAI_BASE: &str = "https://api.openai.com/v1";
const RESERVED_PARAMS: [&str; 3] = ["self", "params", "__class__"];

// The trusted values a deployment hands to its provider; none of them come from the request body.
#[derive(Debug, Clone)]
pub struct Deployment {
    pub model: String,
    pub api_base: String,
    pub endpoint: Url,
    pub api_key: Option<SecretString>,
    pub authorization: Option<SecretString>,
    pub organization: Option<String>,
    pub timeout: Duration,
    pub max_retries: Option<u32>,
    pub drop_params: Option<DropParams>,
    // the model_list entry's litellm_params.additional_drop_params
    pub additional_drop_params: Vec<String>,
    // litellm.drop_params; a call's own drop_params never turns the 422 resend on
    pub global_drop_params: bool,
    pub provider_affinity_header: Option<String>,
    pub anthropic: Option<Arc<AnthropicRuntime>>,
}

impl Deployment {
    #[must_use]
    pub fn from_chat(deployment: &ChatDeployment) -> Self {
        let model = match &deployment.provider {
            DeploymentProvider::OpenAi { model } => model.clone(),
            DeploymentProvider::Unsupported { model } => stripped_model(model),
        };
        Self {
            model,
            api_base: deployment.api_base.clone(),
            endpoint: deployment.endpoint.clone(),
            api_key: deployment.api_key.clone(),
            authorization: deployment.authorization.clone(),
            organization: deployment.organization.clone(),
            timeout: deployment.timeout,
            max_retries: None,
            drop_params: deployment.drop_policy.drop_params.map(DropParams::Flag),
            additional_drop_params: deployment.drop_policy.additional_drop_params.clone(),
            global_drop_params: deployment.global_drop_params,
            provider_affinity_header: None,
            anthropic: None,
        }
    }

    pub fn from_params(mut params: IndexMap<String, JsonValue>) -> Result<Self, DeploymentError> {
        for reserved in RESERVED_PARAMS {
            params.shift_remove(reserved);
        }
        let model = parse::text(params.shift_remove("model"), "model")?
            .ok_or(DeploymentError::MissingModel)?;
        let api_base = parse::text(params.shift_remove("api_base"), "api_base")?
            .unwrap_or_else(|| String::from(DEFAULT_OPENAI_BASE));
        let url = Url::parse(&api_base).map_err(|source| DeploymentError::ApiBase {
            value: api_base.clone(),
            source,
        })?;
        let endpoint = chat_endpoint(url);
        let api_key =
            parse::text(params.shift_remove("api_key"), "api_key")?.map(SecretString::from);
        let organization = parse::text(params.shift_remove("organization"), "organization")?;
        let timeout = parse::seconds(params.shift_remove("timeout"))?;
        let max_retries = parse::retries(params.shift_remove("max_retries"))?;
        let drop_params = parse::drop_params(params.shift_remove("drop_params"));
        let provider_affinity_header =
            parse::affinity_header(params.shift_remove("provider_affinity_header"))?;
        Ok(Self {
            model,
            api_base,
            endpoint,
            api_key,
            authorization: None,
            organization,
            timeout,
            max_retries,
            drop_params,
            additional_drop_params: Vec::new(),
            global_drop_params: false,
            provider_affinity_header,
            anthropic: None,
        })
    }

    // url userinfo becomes a Basic header that replaces the bearer, exactly like HTTPX's auth flow
    #[must_use]
    pub fn credential(&self) -> Option<SecretString> {
        self.authorization.clone().or_else(|| {
            self.api_key
                .as_ref()
                .map(|key| SecretString::from(format!("Bearer {}", key.expose_secret())))
        })
    }

    #[must_use]
    pub fn endpoint_for(&self, resource: &str) -> Url {
        let mut url = self.endpoint.clone();
        let path = format!(
            "{}/{}",
            url.path().trim_end_matches("/chat/completions"),
            resource
        );
        url.set_path(&path);
        url
    }
}

// only a prefix that names a provider comes off; an org/model name keeps its slash
fn stripped_model(litellm_model: &str) -> String {
    let composition = Arc::new(Composition::default());
    litellm_model
        .split_once('/')
        .filter(|(slug, _model)| Provider::from_slug(slug, &composition).is_ok())
        .map_or_else(|| litellm_model.to_owned(), |(_, model)| model.to_owned())
}

fn chat_endpoint(mut url: Url) -> Url {
    let path = format!("{}/chat/completions", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url
}

#[derive(Debug, Error)]
pub enum DeploymentError {
    #[error("the deployment params name no model")]
    MissingModel,
    #[error("deployment field {field} must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("deployment max_retries {value:?} is not an integer")]
    MaxRetries { value: String },
    #[error("deployment api_base {value:?} is not a URL: {source}")]
    ApiBase {
        value: String,
        #[source]
        source: url::ParseError,
    },
    #[error("deployment timeout {value} seconds cannot be represented")]
    Timeout {
        value: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("deployment provider_affinity_header {value:?} cannot be used: {source}")]
    ProviderAffinityHeader {
        value: String,
        #[source]
        source: AffinityError,
    },
}

impl From<TransportError> for OperationError {
    fn from(source: TransportError) -> Self {
        match source {
            TransportError::ConnectTimeout(_) | TransportError::Deadline(_) => Self::Timeout,
            TransportError::BodyTooLarge { limit } | TransportError::HeadersTooLarge { limit } => {
                Self::TooLarge { limit }
            }
            TransportError::Rejected(_)
            | TransportError::Build(_)
            | TransportError::Connect(_)
            | TransportError::Request(_)
            | TransportError::Body(_)
            | TransportError::InvalidHeader { .. }
            | TransportError::MultipartContentType { .. }
            | TransportError::MultipartBoundaryHeader { .. }
            | TransportError::MultipartBoundary { .. }
            | TransportError::Mock(_)
            | TransportError::Closed
            | TransportError::DrainTimeout { .. }
            | TransportError::TooManyRedirects { .. }
            | TransportError::RedirectTarget { .. }
            | TransportError::RedirectUrl { .. }
            | TransportError::RedirectUserinfo
            | TransportError::RedirectScheme { .. }
            | TransportError::InsecureRedirect
            | TransportError::RedirectBody => Self::Http { source },
        }
    }
}

mod parse;
