use std::str::from_utf8;
use std::time::{Duration, Instant};

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::transformation::{BaseEvalsAPIConfig, EvalsHttpResponse};
use betterlite_openai::openai_base_config::{Headers, Object};

use crate::api::vector_store_http_write::error::{display_facts, header_text};

use super::convert::{self, SendFacts};
use super::error::EvalHttpError;
use super::logging;
use super::wire;

// The provider config these handlers call; the gateway's futures need it borrowed as Send and Sync
pub type EvalConfig<'call> = &'call (dyn BaseEvalsAPIConfig<Error = EvalsError> + Send + Sync);

// What a handler holds fixed across its call: the provider config, the pooled client, the URL the transform built and the validated headers
pub struct EvalConnection<'call> {
    pub config: EvalConfig<'call>,
    pub client: &'call ProxiedClient,
    pub url: &'call str,
    pub headers: &'call Headers,
}

// The HTTP method of one call; LiteLLM's get lets any answer through, while post and delete raise for a status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalMethod {
    Get,
    Post,
    Delete,
}

// The parts that differ per handler; complete_input_dict is what the pre-call logs, which is not always what the wire carries
pub struct EvalExchange<'call> {
    pub method: EvalMethod,
    pub body: Option<&'call Object>,
    pub query: Option<&'call Object>,
    // The query the pre-call logs under `params`; only list_runs has one, and it is the same object it sends as query
    pub params: Option<&'call Object>,
    pub timeout: Option<Duration>,
    pub input: &'call str,
    pub complete_input_dict: Option<&'call Object>,
}

// The reply a transform is handed: the upstream status, headers and text as httpx's response exposes them
#[derive(Debug)]
pub struct EvalReply {
    pub status_code: u16,
    pub headers: Headers,
    pub text: String,
}

impl EvalReply {
    // httpx hands the transform the decoded text and the received headers; a body that is not UTF-8 has no text form and is named by its size
    pub(super) fn from_upstream(answer: &UpstreamResponse) -> Self {
        let headers = answer
            .headers
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
            .collect();
        let text = match from_utf8(&answer.body) {
            Ok(decoded) => decoded.to_owned(),
            Err(_invalid) => format!("{} bytes that are not UTF-8", answer.body.len()),
        };
        Self {
            status_code: answer.status,
            headers,
            text,
        }
    }

    #[must_use]
    pub const fn raw_response(&self) -> EvalsHttpResponse<'_> {
        EvalsHttpResponse {
            status_code: self.status_code,
            headers: &self.headers,
            text: self.text.as_str(),
        }
    }
}

// One buffered provider call that follows the redirects httpx's clients follow
async fn send(
    connection: &EvalConnection<'_>,
    request: OutboundRequest,
) -> Result<UpstreamResponse, TransportError> {
    let limit = connection.client.limits.body_bytes;
    connection
        .client
        .send_following(request)
        .await?
        .buffered(limit)
        .await
}

// The four steps every eval handler runs: the pre-call, the send, the status conversion post and delete make, and the reply the transform reads
pub async fn eval_exchange(
    connection: &EvalConnection<'_>,
    exchange: &EvalExchange<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<EvalReply, EvalHttpError> {
    logging::record_pre_call(connection, exchange, log)?;
    let started = Instant::now();
    let request = wire::outbound(connection, exchange).map_err(|failure| {
        convert::failed(connection.config, &display_facts(failure.to_string()))
    })?;
    let deadline = request.deadline;
    let reply = match send(connection, request).await {
        Ok(answer) => answer,
        Err(error) => {
            let facts = SendFacts {
                method: exchange.method,
                deadline,
                elapsed: started.elapsed(),
            };
            return Err(convert::send_failed(connection.config, &facts, &error));
        }
    };
    let checked = if matches!(exchange.method, EvalMethod::Post | EvalMethod::Delete) {
        convert::check_status(connection.config, reply)?
    } else {
        reply
    };
    Ok(EvalReply::from_upstream(&checked))
}
