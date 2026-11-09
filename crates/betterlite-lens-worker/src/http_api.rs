use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
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

struct Call<'call> {
    verb: Verb,
    path: &'call str,
    query: &'call str,
    body: Option<Vec<u8>>,
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

    async fn exchange(&self, call: Call<'_>) -> Result<Answer, WorkerError> {
        let client = &self.client.client;
        let url = format!("{}{}{}", self.base_url, call.path, call.query);
        let mut request = match (call.verb, call.body) {
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
        let sent = client.execute(request);
        let mut response = sent
            .await
            .map_err(|source| transport_failure(source.is_timeout()))?;
        let status = success_status(response.status().as_u16(), call.path)?;
        let mut received = Vec::new();
        loop {
            let pending = response.chunk();
            let chunk = pending
                .await
                .map_err(|source| transport_failure(source.is_timeout()))?;
            let Some(bytes) = chunk else { break };
            extend_body(&mut received, &bytes, self.client.limits.body_bytes)?;
        }
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

const fn transport_failure(timed_out: bool) -> WorkerError {
    if timed_out {
        WorkerError::Timeout
    } else {
        WorkerError::Transport
    }
}

fn extend_body(received: &mut Vec<u8>, chunk: &[u8], limit: usize) -> Result<(), WorkerError> {
    if received.len().saturating_add(chunk.len()) > limit {
        return Err(WorkerError::Decode(format!(
            "the response body is larger than {limit} bytes"
        )));
    }
    received.extend_from_slice(chunk);
    Ok(())
}

impl LensApi for HttpApi {
    async fn claim(&self) -> Result<Option<Claim>, WorkerError> {
        let answer = self
            .exchange(Call {
                verb: Verb::Post,
                path: "/lens/worker/claim",
                query: "?protocol_version=2",
                body: None,
            })
            .await?;
        if answer.body.trim_ascii() == b"null".as_slice() {
            return Ok(None);
        }
        decode(&answer.body)
            .map(Some)
            .map_err(|error| WorkerError::Decode(error.to_string()))
    }

    async fn sample(&self, prefix: &str) -> Result<Sample, WorkerError> {
        let path = format!("{prefix}/sample");
        let answer = self
            .exchange(Call {
                verb: Verb::Get,
                path: &path,
                query: "",
                body: None,
            })
            .await?;
        decode(&answer.body).map_err(|error| WorkerError::Decode(error.to_string()))
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
        let answer = self
            .exchange(Call {
                verb: Verb::Get,
                path: &path,
                query: &query,
                body: None,
            })
            .await?;
        decode(&answer.body).map_err(|error| WorkerError::Decode(error.to_string()))
    }

    async fn model(&self, prefix: &str, request: ModelRequest) -> Result<ModelResult, WorkerError> {
        let path = format!("{prefix}/model");
        let body =
            serde_json::to_vec(&request).map_err(|error| WorkerError::Decode(error.to_string()))?;
        let answer = self
            .exchange(Call {
                verb: Verb::Post,
                path: &path,
                query: "",
                body: Some(body),
            })
            .await?;
        decode(&answer.body).map_err(|error| WorkerError::Decode(error.to_string()))
    }

    async fn progress(&self, prefix: &str, progress: Progress) -> Result<(), WorkerError> {
        let path = format!("{prefix}/progress");
        let body = serde_json::to_vec(&progress)
            .map_err(|error| WorkerError::Decode(error.to_string()))?;
        self.exchange(Call {
            verb: Verb::Post,
            path: &path,
            query: "",
            body: Some(body),
        })
        .await?;
        Ok(())
    }

    async fn heartbeat(&self, prefix: &str) -> Result<(), WorkerError> {
        let path = format!("{prefix}/heartbeat");
        self.exchange(Call {
            verb: Verb::Post,
            path: &path,
            query: "",
            body: None,
        })
        .await?;
        Ok(())
    }

    async fn result(&self, prefix: &str, result: &JobResult) -> Result<ResultStatus, WorkerError> {
        let path = format!("{prefix}/result");
        let body =
            serde_json::to_vec(result).map_err(|error| WorkerError::Decode(error.to_string()))?;
        let answer = self
            .exchange(Call {
                verb: Verb::Post,
                path: &path,
                query: "",
                body: Some(body),
            })
            .await?;
        Ok(ResultStatus {
            status: answer.status,
        })
    }
}

#[path = "http_api/tests.rs"]
mod tests;
