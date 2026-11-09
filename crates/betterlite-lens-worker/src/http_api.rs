use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use betterlite_http::proxy::timed_out;
use betterlite_lens::models::decode::decode;
use betterlite_lens::models::execution::{ExecutionContent, Sample};
use betterlite_lens::models::messages::{Claim, JobResult, ModelRequest, ModelResult, Progress};
use secrecy::{ExposeSecret as _, SecretString};
use url::form_urlencoded;

use crate::api::{LensApi, ResultStatus, WorkerError};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

pub struct HttpApi {
    base_url: String,
    token: SecretString,
    client: ProxiedClient,
}

enum Verb {
    Get,
    Post,
}

struct Answer {
    status: u16,
    body: Vec<u8>,
}

impl HttpApi {
    #[must_use]
    pub fn new(base_url: &str, token: SecretString, client: ProxiedClient) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            token,
            client,
        }
    }

    async fn exchange(
        &self,
        verb: Verb,
        path: &str,
        query: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Answer, WorkerError> {
        let client = &self.client.client;
        let url = format!("{}{}{}", self.base_url, path, query);
        let mut request = match (verb, body) {
            (Verb::Get, _) => client.get(url),
            (Verb::Post, Some(bytes)) => client
                .post(url)
                .header("content-type", "application/json")
                .body(bytes),
            (Verb::Post, None) => client.post(url),
        }
        .timeout(REQUEST_TIMEOUT)
        .header(
            "authorization",
            format!("Bearer {}", self.token.expose_secret()),
        )
        .build()
        .map_err(|_source| WorkerError::Transport)?;
        if let Some(value) = request.headers_mut().get_mut("authorization") {
            value.set_sensitive(true);
        }
        let mut response = self.client.execute(request).await.map_err(http_failure)?;
        let status = success_status(response.status().as_u16(), path)?;
        let received = read_bounded(&mut response, self.client.limits.body_bytes)
            .await
            .map_err(|error| match error {
                ReadError::TooLarge { limit } => {
                    WorkerError::Decode(format!("the response body is larger than {limit} bytes"))
                }
                ReadError::Failed(source) => transport_failure(source.is_timeout()),
            })?;
        Ok(Answer {
            status,
            body: received,
        })
    }
}

fn success_status(status: u16, path: &str) -> Result<u16, WorkerError> {
    if (200..300).contains(&status) {
        Ok(status)
    } else {
        Err(WorkerError::Status {
            path: path.to_owned(),
            status,
        })
    }
}

fn http_failure(error: HttpError) -> WorkerError {
    if matches!(timed_out(error), HttpError::Timeout) {
        WorkerError::Timeout
    } else {
        WorkerError::Transport
    }
}

const fn transport_failure(timed_out: bool) -> WorkerError {
    if timed_out {
        WorkerError::Timeout
    } else {
        WorkerError::Transport
    }
}

impl LensApi for HttpApi {
    async fn claim(&self) -> Result<Option<Claim>, WorkerError> {
        let answer = self
            .exchange(
                Verb::Post,
                "/lens/worker/claim",
                "?protocol_version=2",
                None,
            )
            .await?;
        if answer.body.trim_ascii() == b"null".as_slice() {
            return Ok(None);
        }
        decode(&answer.body).map(Some).map_err(WorkerError::from)
    }

    async fn sample(&self, prefix: &str) -> Result<Sample, WorkerError> {
        let path = format!("{prefix}/sample");
        let answer = self.exchange(Verb::Get, &path, "", None).await?;
        decode(&answer.body).map_err(WorkerError::from)
    }

    async fn content(
        &self,
        prefix: &str,
        execution_id: &str,
        cursor: &str,
        offset: i64,
    ) -> Result<ExecutionContent, WorkerError> {
        let path = format!("{prefix}/content");
        let offset_text = offset.to_string();
        let query = format!(
            "?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("execution_id", execution_id)
                .append_pair("cursor", cursor)
                .append_pair("offset", &offset_text)
                .finish()
        );
        let answer = self.exchange(Verb::Get, &path, &query, None).await?;
        decode(&answer.body).map_err(WorkerError::from)
    }

    async fn model(&self, prefix: &str, request: ModelRequest) -> Result<ModelResult, WorkerError> {
        let path = format!("{prefix}/model");
        let body = serde_json::to_vec(&request).map_err(WorkerError::from)?;
        let answer = self.exchange(Verb::Post, &path, "", Some(body)).await?;
        decode(&answer.body).map_err(WorkerError::from)
    }

    async fn progress(&self, prefix: &str, progress: Progress) -> Result<(), WorkerError> {
        let path = format!("{prefix}/progress");
        let body = serde_json::to_vec(&progress).map_err(WorkerError::from)?;
        self.exchange(Verb::Post, &path, "", Some(body)).await?;
        Ok(())
    }

    async fn heartbeat(&self, prefix: &str) -> Result<(), WorkerError> {
        let path = format!("{prefix}/heartbeat");
        self.exchange(Verb::Post, &path, "", None).await?;
        Ok(())
    }

    async fn result(&self, prefix: &str, result: &JobResult) -> Result<ResultStatus, WorkerError> {
        let path = format!("{prefix}/result");
        let body = serde_json::to_vec(result).map_err(WorkerError::from)?;
        let answer = self.exchange(Verb::Post, &path, "", Some(body)).await?;
        Ok(ResultStatus {
            status: answer.status,
        })
    }
}

#[path = "http_api/tests.rs"]
mod tests;
