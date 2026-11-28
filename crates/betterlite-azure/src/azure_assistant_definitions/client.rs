use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::assistants::decode::decode_assistant_object;
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::assistants::records::{
    AssistantCreate, AssistantDeleted, AssistantObject, AssistantPage,
};
use betterlite_openai::assistants::transport::AssistantsTransport;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::azure_urls::url_parts::{encoded_query, first_query_values};
use crate::credentials::source::TokenReply;
use crate::endpoints::projection::{AzureClientInputs, TokenProvider};

const API_KEY: HeaderName = HeaderName::from_static("api-key");
const OPENAI_BETA: HeaderName = HeaderName::from_static("openai-beta");
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
// Python's quote(safe="!$&'()*+,;=:@") leaves these marks alone, and NON_ALPHANUMERIC encodes all of them.
const SAFE_PATH_MARKS: [(&str, &str); 17] = [
    ("%21", "!"),
    ("%24", "$"),
    ("%26", "&"),
    ("%27", "'"),
    ("%28", "("),
    ("%29", ")"),
    ("%2A", "*"),
    ("%2B", "+"),
    ("%2C", ","),
    ("%2D", "-"),
    ("%2E", "."),
    ("%3A", ":"),
    ("%3B", ";"),
    ("%3D", "="),
    ("%40", "@"),
    ("%5F", "_"),
    ("%7E", "~"),
];

// The Azure SDK client a facade call produced or was handed, with what a request needs and the inputs it was built from.
pub struct AzureAssistantsClient<'data, T> {
    pub kind: ClientKind,
    pub inputs: Option<AzureClientInputs<'data>>,
    pub transport: Arc<T>,
    pub base_url: String,
    pub api_version: String,
    pub key: Option<SecretString>,
    pub token: Option<SecretString>,
    pub deadline: Duration,
}

impl<T> fmt::Debug for AzureAssistantsClient<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureAssistantsClient")
            .field("kind", &self.kind)
            .field("inputs", &self.inputs)
            .field("base_url", &self.base_url)
            .field("api_version", &self.api_version)
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

impl<'data, T: AssistantsTransport> AzureAssistantsClient<'data, T> {
    pub fn new(
        kind: ClientKind,
        inputs: AzureClientInputs<'data>,
        environment: &dyn EnvironmentLookup,
        transport: Arc<T>,
    ) -> Result<Self, DefinitionsError> {
        let (key, token) = resolve_credentials(&inputs, environment)?;
        let base_url = resolve_base_url(&inputs, environment)?;
        let deadline = resolve_deadline(inputs.timeout)?;
        let api_version = inputs.api_version.clone();
        Ok(Self {
            kind,
            api_version,
            base_url,
            key,
            token,
            inputs: Some(inputs),
            transport,
            deadline,
        })
    }

    pub async fn list(&self) -> Result<AssistantPage, DefinitionsError> {
        let request = self.outbound(Method::GET, "assistants", RequestBody::Empty)?;
        let body = self.send_checked(request).await?;
        Ok(decode_assistant_object(&body)?)
    }

    pub async fn create(
        &self,
        data: &AssistantCreate,
    ) -> Result<AssistantObject, DefinitionsError> {
        let body = serde_json::to_vec(data).map_err(|source| AssistantsError::Encode { source })?;
        let request = self.outbound(Method::POST, "assistants", RequestBody::Json(body))?;
        let reply = self.send_checked(request).await?;
        Ok(decode_assistant_object(&reply)?)
    }

    pub async fn delete(&self, assistant_id: &str) -> Result<AssistantDeleted, DefinitionsError> {
        if assistant_id.is_empty() {
            return Err(DefinitionsError::Assistants(
                AssistantsError::EmptyAssistantId,
            ));
        }
        if assistant_id == "." || assistant_id == ".." {
            return Err(DefinitionsError::Assistants(AssistantsError::DotSegment {
                segment: assistant_id.to_owned(),
            }));
        }
        let escaped = quote_path_segment(assistant_id);
        let tail = format!("assistants/{escaped}");
        let mut request = self.outbound(Method::DELETE, &tail, RequestBody::Empty)?;
        request
            .headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let body = self.send_checked(request).await?;
        Ok(decode_assistant_object(&body)?)
    }

    pub(crate) fn outbound(
        &self,
        method: Method,
        tail: &str,
        body: RequestBody,
    ) -> Result<OutboundRequest, DefinitionsError> {
        let joined = if self.base_url.ends_with('/') {
            format!("{}{tail}", self.base_url)
        } else {
            format!("{}/{tail}", self.base_url)
        };
        let mut url = Url::parse(&joined).map_err(|source| DefinitionsError::BaseUrl { source })?;
        let query = merged_query(url.query(), &self.api_version);
        url.set_query(query.as_deref());
        let mut headers = self.auth_headers()?;
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(OPENAI_BETA, HeaderValue::from_static("assistants=v2"));
        Ok(OutboundRequest {
            method,
            url,
            query: Vec::new(),
            headers,
            body,
            deadline: self.deadline,
        })
    }

    // The SDK sends a static token alone, the provider token next to a real key, otherwise only the key.
    fn auth_headers(&self) -> Result<HeaderMap, DefinitionsError> {
        let mut headers = HeaderMap::new();
        let mut bearer = self.token.clone();
        if bearer.is_none()
            && let Some(provider) = self
                .inputs
                .as_ref()
                .and_then(|inputs| inputs.token_provider.as_ref())
        {
            bearer = Some(provider_token(provider)?);
        }
        let api_key = self
            .key
            .as_ref()
            .filter(|value| !value.expose_secret().is_empty());
        if let Some(token) = bearer.as_ref() {
            let mut header = HeaderValue::try_from(format!("Bearer {}", token.expose_secret()))
                .map_err(|source| AssistantsError::Header { source })?;
            header.set_sensitive(true);
            headers.insert(AUTHORIZATION, header);
        }
        if self.token.is_none()
            && let Some(key) = api_key
        {
            let mut header = HeaderValue::try_from(key.expose_secret())
                .map_err(|source| AssistantsError::Header { source })?;
            header.set_sensitive(true);
            headers.insert(API_KEY, header);
        }
        Ok(headers)
    }

    pub(crate) async fn send_checked(
        &self,
        request: OutboundRequest,
    ) -> Result<Vec<u8>, DefinitionsError> {
        let response = self
            .transport
            .send(request)
            .await
            .map_err(|source| AssistantsError::Transport { source })?;
        if response.status >= 400 {
            return Err(DefinitionsError::Assistants(AssistantsError::Status {
                status: response.status,
                body: response.body,
            }));
        }
        Ok(response.body)
    }
}

// The SDK reads its key and token from the environment when the constructor arguments are absent.
fn resolve_credentials(
    inputs: &AzureClientInputs<'_>,
    environment: &dyn EnvironmentLookup,
) -> Result<(Option<SecretString>, Option<SecretString>), DefinitionsError> {
    let key = inputs.api_key.clone().or_else(|| {
        environment
            .lookup("AZURE_OPENAI_API_KEY")
            .map(SecretString::from)
    });
    let token = inputs.azure_ad_token.clone().or_else(|| {
        environment
            .lookup("AZURE_OPENAI_AD_TOKEN")
            .map(SecretString::from)
    });
    // An explicit empty key builds nothing: the SDK's base client refuses a falsy key even with a token (openai/_client.py#L223-L233).
    let empty_key = key
        .as_ref()
        .is_some_and(|value| value.expose_secret().is_empty());
    if empty_key || (key.is_none() && token.is_none() && inputs.token_provider.is_none()) {
        return Err(DefinitionsError::MissingCredentials);
    }
    Ok((key, token))
}

// A base_url is used as given; a bare azure_endpoint gets /openai after its trailing slashes are trimmed.
fn resolve_base_url(
    inputs: &AzureClientInputs<'_>,
    environment: &dyn EnvironmentLookup,
) -> Result<String, DefinitionsError> {
    match (&inputs.endpoint.base_url, &inputs.endpoint.azure_endpoint) {
        (Some(base), _) => Ok(base.clone()),
        (None, Some(endpoint)) => Ok(format!("{}/openai", endpoint.trim_end_matches('/'))),
        (None, None) => {
            let endpoint = environment
                .lookup("AZURE_OPENAI_ENDPOINT")
                .ok_or(DefinitionsError::MissingEndpoint)?;
            Ok(format!("{}/openai", endpoint.trim_end_matches('/')))
        }
    }
}

fn resolve_deadline(timeout: Option<f64>) -> Result<Duration, DefinitionsError> {
    let Some(seconds) = timeout else {
        return Ok(DEFAULT_TIMEOUT);
    };
    Duration::try_from_secs_f64(seconds).map_err(|_source| DefinitionsError::Timeout { seconds })
}

fn provider_token(provider: &TokenProvider<'_>) -> Result<SecretString, DefinitionsError> {
    match provider.0.token() {
        Ok(TokenReply::Text(token)) if !token.expose_secret().is_empty() => Ok(token),
        Ok(TokenReply::Text(_) | TokenReply::NonText { .. }) => {
            Err(DefinitionsError::ProviderReply)
        }
        Err(failure) => Err(DefinitionsError::Provider {
            class: failure.class,
            message: failure.message,
        }),
    }
}

// A relative path joins the base URL's raw path, so a base URL query swallows the tail exactly as the SDK's does.
fn merged_query(query: Option<&str>, api_version: &str) -> Option<String> {
    let mut pairs = first_query_values(query);
    match pairs.iter_mut().find(|(name, _)| name == "api-version") {
        Some((_, value)) => api_version.clone_into(value),
        None => pairs.push(("api-version".to_owned(), api_version.to_owned())),
    }
    // urlencode drops a pair whose value is empty, so an empty api-version leaves no query at all.
    pairs.retain(|(_, value)| !value.is_empty());
    encoded_query(&pairs)
}

pub(crate) fn quote_path_segment(segment: &str) -> String {
    let mut escaped = utf8_percent_encode(segment, NON_ALPHANUMERIC).to_string();
    for (encoded, mark) in SAFE_PATH_MARKS {
        escaped = escaped.replace(encoded, mark);
    }
    escaped
}
