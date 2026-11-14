use std::error::Error as StdError;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::limits::read_bounded;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::model_armor::decisions::wire::SanitizeResponse;
use crate::model_armor::decisions::{ModelArmorAPIError, build_api_error_detail};
use crate::model_armor::file_scanning::ByteDataType;

pub mod error;
pub mod project;
mod tests;
pub mod tokens;
pub mod wire;

pub use error::{ArmorCallError, ArmorTokenError, TransportError};
pub use tokens::ArmorTokens;
pub use wire::{PromptData, SanitizeBody, Source};

use project::ResolvedProject;

// The response carries findings, not a document, so a megabyte is far beyond any real sanitize answer
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub struct ModelArmorTransport<T: ArmorTokens> {
    pub template_id: Option<String>,
    pub project_id: Option<String>,
    pub location: String,
    pub api_endpoint: Option<String>,
    pub sanitize_error_detail: bool,
    pub timeout: Option<Duration>,
    pub client: reqwest::Client,
    pub tokens: Arc<T>,
    pub resolved_project: ResolvedProject,
}

#[derive(Debug, Clone)]
pub struct ArmorResponse {
    pub body: GuardrailValue,
    pub verdict: SanitizeResponse,
}

impl<T: ArmorTokens> ModelArmorTransport<T> {
    #[must_use]
    pub fn api_endpoint(&self) -> String {
        match self
            .api_endpoint
            .as_deref()
            .filter(|endpoint| !endpoint.is_empty())
        {
            Some(endpoint) => endpoint.to_owned(),
            None => format!("https://modelarmor.{}.rep.googleapis.com", self.location),
        }
    }

    #[must_use]
    pub fn sanitize_url(&self, source: Source) -> String {
        format!(
            "{}/v1/projects/{}/locations/{}/templates/{}:{}",
            self.api_endpoint(),
            self.resolved_project
                .effective(self.project_id.as_deref())
                .unwrap_or_default(),
            self.location,
            self.template_id.as_deref().unwrap_or_default(),
            source.method()
        )
    }

    pub async fn make_model_armor_request(
        &self,
        content: Option<&str>,
        source: Source,
        file: Option<(&[u8], ByteDataType)>,
    ) -> Result<ArmorResponse, ArmorCallError<T::Error>> {
        if self.template_id.is_none() {
            return Err(ArmorCallError::<T::Error>::from(
                TransportError::<T::Error>::MissingTemplate,
            ));
        }
        let (access_token, resolved_project_id) = self
            .tokens
            .access_token()
            .await
            .map_err(TransportError::<T::Error>::Token)?;
        self.resolved_project
            .remember(self.project_id.as_deref(), resolved_project_id.as_deref());
        let url = self.sanitize_url(source);
        let body = request_body::<T::Error>(content, source, file)?;
        self.log_request(&url, &body, file);
        let text = self.post(&url, &body, &access_token).await?;
        parse_response::<T::Error>(&text)
    }

    fn log_request(&self, url: &str, body: &SanitizeBody, file: Option<(&[u8], ByteDataType)>) {
        match file {
            Some((file_bytes, byte_data_type)) => tracing::debug!(
                url = %url,
                byte_data_type = %byte_data_type,
                bytes = file_bytes.len(),
                "Model Armor file request"
            ),
            None if self.sanitize_error_detail => {
                tracing::debug!(url = %url, "Model Armor request");
            }
            None => tracing::debug!(url = %url, body = ?body, "Model Armor request"),
        }
    }

    async fn post(
        &self,
        url: &str,
        body: &SanitizeBody,
        access_token: &SecretString,
    ) -> Result<String, ArmorCallError<T::Error>> {
        let token = access_token.expose_secret();
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(TransportError::<T::Error>::Header)?;
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let mut request = self.client.post(url).headers(headers).json(body);
        if let Some(timeout) = self.timeout {
            request = request.timeout(timeout);
        }
        let mut response = request.send().await.map_err(request_error::<T::Error>)?;
        let status = response.status();
        let bytes = read_bounded(&mut response, MAX_RESPONSE_BYTES)
            .await
            .map_err(TransportError::<T::Error>::Body)?;
        let text = String::from_utf8(bytes).map_err(TransportError::<T::Error>::ResponseText)?;
        if !status.is_success() {
            return Err(self.api_error(status, &text));
        }
        if self.sanitize_error_detail {
            tracing::debug!(status = status.as_u16(), "Model Armor response");
        } else {
            tracing::debug!(status = status.as_u16(), body = %text, "Model Armor response");
        }
        if status != StatusCode::OK {
            return Err(self.api_error(status, &text));
        }
        Ok(text)
    }

    fn api_error(&self, status: StatusCode, text: &str) -> ArmorCallError<T::Error> {
        let detail = build_api_error_detail(status.as_u16(), text, self.sanitize_error_detail);
        tracing::error!(status = status.as_u16(), detail = %detail, "Model Armor API error");
        ArmorCallError::<T::Error>::from(ModelArmorAPIError { detail })
    }
}

fn request_body<E: StdError + Send + Sync + 'static>(
    content: Option<&str>,
    source: Source,
    file: Option<(&[u8], ByteDataType)>,
) -> Result<SanitizeBody, ArmorCallError<E>> {
    if let Some((file_bytes, byte_data_type)) = file {
        return Ok(file_body(file_bytes, byte_data_type, source));
    }
    match content {
        Some(content) => Ok(text_body(content, source)),
        None => Err(ArmorCallError::<E>::from(TransportError::<E>::NoContent)),
    }
}

fn request_error<E: StdError + Send + Sync + 'static>(source: reqwest::Error) -> ArmorCallError<E> {
    if source.is_timeout() {
        ArmorCallError::<E>::from(TransportError::<E>::Timeout(source))
    } else {
        ArmorCallError::<E>::from(TransportError::<E>::Request(source))
    }
}

fn parse_response<E: StdError + Send + Sync + 'static>(
    text: &str,
) -> Result<ArmorResponse, ArmorCallError<E>> {
    let verdict = serde_json::from_str::<SanitizeResponse>(text)
        .map_err(|source| ArmorCallError::<E>::from(TransportError::<E>::ResponseJson(source)))?;
    let body = serde_json::from_str::<GuardrailValue>(text)
        .map_err(|source| ArmorCallError::<E>::from(TransportError::<E>::ResponseJson(source)))?;
    Ok(ArmorResponse { body, verdict })
}

#[must_use]
pub fn text_body(content: &str, source: Source) -> SanitizeBody {
    SanitizeBody::wrap(
        source,
        PromptData::Text(wire::TextPrompt {
            text: content.to_owned(),
        }),
    )
}

#[must_use]
pub fn file_body(file_bytes: &[u8], byte_data_type: ByteDataType, source: Source) -> SanitizeBody {
    SanitizeBody::wrap(
        source,
        PromptData::Bytes(wire::ByteItem {
            byte_item: wire::BytePayload {
                byte_data_type,
                byte_data: STANDARD.encode(file_bytes),
            },
        }),
    )
}
