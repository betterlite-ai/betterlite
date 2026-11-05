use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::openai_credentials::DEFAULT_API_BASE;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::assistants::decode::decode_assistant_object;
use crate::assistants::error::AssistantsError;
use crate::assistants::records::{
    AssistantCreate, AssistantDeleted, AssistantObject, AssistantPage,
};
use crate::assistants::transport::AssistantsTransport;

const OPENAI_BETA: HeaderName = HeaderName::from_static("openai-beta");
const OPENAI_ORGANIZATION: HeaderName = HeaderName::from_static("openai-organization");
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
// The OpenAI SDK quotes a path id with Python's quote(safe="!$&'()*+,;=:@"), so "/" is encoded and the unreserved marks stay.
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b'!')
    .remove(b'$')
    .remove(b'&')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b';')
    .remove(b'=')
    .remove(b':')
    .remove(b'@');

#[derive(Debug, Clone, Default)]
pub struct ClientOptions {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Option<Duration>,
    pub max_retries: Option<u32>,
    pub organization: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Async,
}

// The fields are the keyword arguments the OpenAI constructor received: None means the argument was left out.
pub struct SdkClient<T> {
    pub kind: ClientKind,
    pub api_key: Option<SecretString>,
    pub base_url: Option<String>,
    pub timeout: Option<Duration>,
    pub max_retries: Option<u32>,
    pub organization: Option<String>,
    pub transport: Arc<T>,
}

impl<T> fmt::Debug for SdkClient<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SdkClient")
            .field("kind", &self.kind)
            .field("api_key", &self.api_key)
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .field("max_retries", &self.max_retries)
            .field("organization", &self.organization)
            .finish_non_exhaustive()
    }
}

impl<T: AssistantsTransport> SdkClient<T> {
    fn outbound(
        &self,
        method: Method,
        tail: &str,
        query: Vec<(String, String)>,
        body: RequestBody,
    ) -> Result<OutboundRequest, AssistantsError> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or(AssistantsError::MissingApiKey)?;
        let mut url = Url::parse(self.base_url.as_deref().unwrap_or(DEFAULT_API_BASE))
            .map_err(|source| AssistantsError::BaseUrl { source })?;
        let mut path = url.path().to_owned();
        if !path.ends_with('/') {
            path.push('/');
        }
        url.set_path(&format!("{path}{tail}"));
        let mut headers = HeaderMap::new();
        let mut bearer = HeaderValue::try_from(format!("Bearer {}", api_key.expose_secret()))
            .map_err(|source| AssistantsError::Header { source })?;
        bearer.set_sensitive(true);
        headers.insert(AUTHORIZATION, bearer);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(OPENAI_BETA, HeaderValue::from_static("assistants=v2"));
        if let Some(organization) = &self.organization {
            let value = HeaderValue::try_from(organization.as_str())
                .map_err(|source| AssistantsError::Header { source })?;
            headers.insert(OPENAI_ORGANIZATION, value);
        }
        Ok(OutboundRequest {
            method,
            url,
            query,
            headers,
            body,
            deadline: self.timeout.unwrap_or(DEFAULT_TIMEOUT),
        })
    }

    async fn send_checked(&self, request: OutboundRequest) -> Result<Vec<u8>, AssistantsError> {
        let response = self
            .transport
            .send(request)
            .await
            .map_err(|source| AssistantsError::Transport { source })?;
        if response.status >= 400 {
            return Err(AssistantsError::Status {
                status: response.status,
                body: response.body,
            });
        }
        Ok(response.body)
    }

    pub async fn list(
        &self,
        request_params: Vec<(String, String)>,
    ) -> Result<AssistantPage, AssistantsError> {
        let request = self.outbound(
            Method::GET,
            "assistants",
            request_params,
            RequestBody::Empty,
        )?;
        let body = self.send_checked(request).await?;
        serde_json::from_slice(&body).map_err(AssistantsError::from)
    }

    pub async fn create(&self, data: &AssistantCreate) -> Result<AssistantObject, AssistantsError> {
        let body = serde_json::to_vec(data).map_err(|source| AssistantsError::Encode { source })?;
        let request = self.outbound(
            Method::POST,
            "assistants",
            Vec::new(),
            RequestBody::Json(body),
        )?;
        let reply = self.send_checked(request).await?;
        decode_assistant_object(&reply)
    }

    pub async fn delete(&self, assistant_id: &str) -> Result<AssistantDeleted, AssistantsError> {
        if assistant_id.is_empty() {
            return Err(AssistantsError::EmptyAssistantId);
        }
        if assistant_id == "." || assistant_id == ".." {
            return Err(AssistantsError::DotSegment {
                segment: assistant_id.to_owned(),
            });
        }
        let quoted = utf8_percent_encode(assistant_id, PATH_SEGMENT);
        let mut request = self.outbound(
            Method::DELETE,
            &format!("assistants/{quoted}"),
            Vec::new(),
            RequestBody::Empty,
        )?;
        request
            .headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let body = self.send_checked(request).await?;
        serde_json::from_slice(&body).map_err(AssistantsError::from)
    }
}
