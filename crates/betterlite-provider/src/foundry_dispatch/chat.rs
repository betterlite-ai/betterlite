use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::openai_base_config::Headers;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::AzureAi;
use super::credentials::Globals;
use super::post_call::{self, FoundryLogger};
use crate::openai_dispatch::logging::{LoggedCall, logged_json};
use crate::operation::{OperationError, OperationResult};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::transform_retry::send_with_retry;

pub mod decode;
pub mod prepare;

pub const EXTRA_HEADERS_FIELD: &str = "extra_headers";

pub struct Call<'data> {
    pub model: &'data str,
    pub request: &'data ChatRequest,
    // main.py's `extra_headers` kwarg, which it moves into the optional parameter map
    pub extra_headers: Option<&'data Headers>,
    // main.py's `headers or litellm.headers`, which the provider call sends and the failure log names
    pub headers: Option<&'data Headers>,
    pub api_base: Option<&'data str>,
    pub api_key: Option<&'data SecretString>,
    pub deployment: &'data Deployment,
    pub stream: bool,
}

// A supplied answer lets a case decode an upstream body without a socket; a client sends a real request
pub enum Answer<'data> {
    Supplied {
        status: u16,
        headers: HeaderMap,
        body: Vec<u8>,
    },
    Client(&'data ProxiedClient),
}

pub struct Dispatch<'data> {
    pub call: Call<'data>,
    pub globals: Globals<'data>,
    pub answer: Answer<'data>,
}

pub struct Outcome {
    pub prepared: PreparedRequest,
    pub result: OperationResult,
}

impl Dispatch<'_> {
    pub async fn run(&self, logger: &mut dyn FoundryLogger) -> Result<Outcome, OperationError> {
        // main.py's try wraps the provider call alone; a preparation error never reaches the failure log
        let plan = prepare::prepare(&self.call, &self.globals).map_err(OperationError::from)?;
        match self.execute(&plan.prepared).await {
            Ok(result) => {
                // main.py logs a normal success only when optional_params["stream"] is truthy
                if self.call.stream {
                    logger.post_call(record(
                        &plan,
                        Json::Object(IndexMap::from([(String::from("stream"), Json::Bool(true))])),
                    ));
                }
                Ok(Outcome {
                    prepared: plan.prepared,
                    result,
                })
            }
            Err(error) => {
                logger.post_call(record(&plan, Json::Text(error.to_string())));
                Err(error)
            }
        }
    }

    async fn execute(&self, prepared: &PreparedRequest) -> Result<OperationResult, OperationError> {
        let upstream = match &self.answer {
            Answer::Supplied {
                status,
                headers,
                body,
            } => Upstream::Buffered(UpstreamResponse {
                status: *status,
                headers: headers.clone(),
                body: body.clone(),
            }),
            Answer::Client(client) => {
                send_with_retry(client, prepared, self.call.deployment, &AzureAi).await?
            }
        };
        decode::decode(prepared, upstream)
    }
}

fn record(plan: &prepare::Plan, report: Json) -> LoggedCall {
    LoggedCall {
        input: logged_json(&plan.messages),
        original_response: report,
        headers: post_call::headers_json(plan.headers.as_ref()),
    }
}
