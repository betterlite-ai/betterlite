use std::collections::BTreeMap;

use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::AktoError;
use super::payload::AktoPayload;
use super::response::AktoAnswer;
use super::{AKTO_CONNECTOR_NAME, AktoMirroring, HTTP_PROXY_PATH};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[must_use]
pub fn build_query_params(guardrails: bool, ingest_data: bool) -> BTreeMap<String, String> {
    let mut params =
        BTreeMap::from([("akto_connector".to_owned(), AKTO_CONNECTOR_NAME.to_owned())]);
    if guardrails {
        params.insert("guardrails".to_owned(), "true".to_owned());
    }
    if ingest_data {
        params.insert("ingest_data".to_owned(), "true".to_owned());
    }
    params
}

pub fn prepare_headers(api_key: &SecretString) -> Result<HeaderMap, AktoError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let mut credential = HeaderValue::from_str(api_key.expose_secret())
        .map_err(|source| AktoError::InvalidHeader { source })?;
    credential.set_sensitive(true);
    headers.insert(AUTHORIZATION, credential);
    Ok(headers)
}

impl AktoMirroring {
    pub async fn send_request(
        &self,
        guardrails: bool,
        ingest_data: bool,
        payload: &AktoPayload,
    ) -> Result<AktoAnswer, AktoError> {
        let endpoint = format!("{}{HTTP_PROXY_PATH}", self.base_url);
        let body = serde_json::to_string(payload).map_err(|source| AktoError::Encode { source })?;
        let response = self
            .client
            .post(endpoint)
            .query(&build_query_params(guardrails, ingest_data))
            .headers(prepare_headers(&self.api_key)?)
            .timeout(self.timeout)
            .body(body)
            .send()
            .await
            .map_err(|source| AktoError::Transport { source })?;
        read_answer(response).await
    }

    pub async fn fire_and_forget_request(
        &self,
        guardrails: bool,
        ingest_data: bool,
        payload: &AktoPayload,
    ) {
        match self.send_request(guardrails, ingest_data, payload).await {
            Ok(answer) if answer.status == StatusCode::OK => {}
            Ok(answer) => tracing::error!(
                status = answer.status.as_u16(),
                "Akto fire-and-forget returned a non-200 status"
            ),
            Err(error) => {
                tracing::error!(error = %error, "Akto fire-and-forget error");
            }
        }
    }

    pub fn spawn_fire_and_forget(&self, guardrails: bool, ingest_data: bool, payload: AktoPayload) {
        let guardrail = self.clone();
        self.tasks.spawn(async move {
            guardrail
                .fire_and_forget_request(guardrails, ingest_data, &payload)
                .await;
        });
    }
}

async fn read_answer(mut response: reqwest::Response) -> Result<AktoAnswer, AktoError> {
    let status = response.status();
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|source| AktoError::Transport { source })?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(AktoError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(body).map_err(|source| AktoError::ResponseEncoding { source })?;
    Ok(AktoAnswer { status, body: text })
}
