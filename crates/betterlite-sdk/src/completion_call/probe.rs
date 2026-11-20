use super::target::CompletionTarget;
use crate::call_setup::RequestMap;
use crate::call_wrapper::error::CallError;
use crate::call_wrapper::target::CallTarget;
use betterlite_core::environment_validation::probe::{ModelProbe, ProbeCompletion};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::header::AUTHORIZATION;
use http::{HeaderMap, HeaderValue, Method};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use std::future::Future;
use std::time::Duration;
use thiserror::Error;
use url::Url;
#[derive(Debug, Error)]
pub enum ProbeFailure {
    #[error("the probe has no api_base to send a model lookup to")]
    MissingApiBase,
    #[error("the api_base {api_base} cannot hold a model path")]
    ApiBase { api_base: String },
    #[error("the model lookup path could not be written")]
    Path,
    #[error("the bearer header could not be written")]
    Header,
    #[error("the model lookup answered with status {status}")]
    Status { status: u16 },
    #[error("the model lookup could not be sent: {source}")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error(transparent)]
    Call(#[from] CallError),
}
// B5364's real probe: the model lookup openai.models.retrieve makes, and a real completion
pub struct CompletionProbe<'probe, 'ports> {
    pub client: &'probe ProxiedClient,
    pub completion: &'probe CompletionTarget<'ports>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub timeout: Duration,
}
impl CompletionProbe<'_, '_> {
    async fn retrieve(&self, model: &str) -> Result<(), ProbeFailure> {
        let Some(base) = self.api_base.as_deref() else {
            return Err(ProbeFailure::MissingApiBase);
        };
        let mut url = Url::parse(base).map_err(|_error| ProbeFailure::ApiBase {
            api_base: base.to_owned(),
        })?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_error| ProbeFailure::ApiBase {
                    api_base: base.to_owned(),
                })?;
            segments.pop_if_empty();
            segments.push("models");
            segments.push(model);
        }
        let mut headers = HeaderMap::new();
        if let Some(key) = &self.api_key {
            let value = HeaderValue::from_str(&format!("Bearer {}", key.expose_secret()))
                .map_err(|_error| ProbeFailure::Header)?;
            headers.insert(AUTHORIZATION, value);
        }
        let request = OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Empty,
            deadline: self.timeout,
        };
        let response = self
            .client
            .send_following(request)
            .await
            .map_err(|source| ProbeFailure::Transport { source })?
            .buffered(self.client.limits.body_bytes)
            .await
            .map_err(|source| ProbeFailure::Transport { source })?;
        if (200..300).contains(&response.status) {
            Ok(())
        } else {
            Err(ProbeFailure::Status {
                status: response.status,
            })
        }
    }
}
impl ModelProbe for CompletionProbe<'_, '_> {
    type Failure = ProbeFailure;
    fn retrieve_openai_model(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        async move { self.retrieve(model).await }
    }
    fn complete(
        &self,
        request: &ProbeCompletion<'_>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        async move {
            let mut map = RequestMap::new();
            map.insert(String::from("model"), Json::Text(request.model.to_owned()));
            map.insert(
                String::from("messages"),
                Json::List(vec![Json::Object(IndexMap::from([
                    (String::from("role"), Json::Text(String::from("user"))),
                    (
                        String::from("content"),
                        Json::Text(request.user_message.to_owned()),
                    ),
                ]))]),
            );
            if let Some(tokens) = request.max_tokens {
                map.insert(
                    String::from("max_tokens"),
                    Json::Number(serde_json::Number::from(tokens)),
                );
            }
            if let Some(key) = request.api_key {
                map.insert(
                    String::from("api_key"),
                    Json::Text(key.expose_secret().to_owned()),
                );
            }
            self.completion.call(map, None).await?;
            Ok(())
        }
    }
}
