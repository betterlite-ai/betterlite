use std::collections::BTreeMap;
use std::time::UNIX_EPOCH;

use betterlite_a2a::agentcore_transform::params::LitellmParams;
use betterlite_a2a::agentcore_transform::runtime::{
    AgentCoreRuntime, HttpRequest, HttpResponse, RpcMethod, SigningRequest, StreamedResponse,
};
use betterlite_a2a::agentcore_transform::{
    AgentCoreError, AgentCoreHandler, BedrockAgentCoreConfig, ConfigCall, HandlerCall,
    SignedRequest, SignedRequestCall, TransformError, get_url_and_signed_request,
    parse_sse_events,
};
use betterlite_a2a::completion_bridge::EventSource;
use betterlite_a2a::completion_bridge::wire::MessageSendParams;
use betterlite_bedrock::agentcore_request::error::AgentCoreRequestError;
use betterlite_bedrock::agentcore_request::identifiers::{runtime_session_id, runtime_user_id};
use betterlite_bedrock::agentcore_request::signing::{SignRequest, sign_request};
use betterlite_bedrock::agentcore_request::url::complete_url;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::model_routes::{ParamMap, ParamValue};
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_core::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use futures_util::stream::{self, Iter};
use futures_util::{Stream, StreamExt as _};
use secrecy::SecretString;
use serde::Deserialize;
use uuid::Uuid;

use crate::compare::helper::HelperError;
use crate::helpers::{Helper, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Clone, Deserialize)]
struct Answer {
    status: u16,
    body: AgentJson,
}

#[derive(Clone, Deserialize)]
struct StreamAnswer {
    status: u16,
    content_type: String,
    lines: Vec<String>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    SignedRequest {
        request_id: String,
        params: MessageSendParams,
        litellm_params: BTreeMap<String, AgentJson>,
        #[serde(default)]
        agent_extra_headers: Option<BTreeMap<String, String>>,
    },
    ParseSse {
        lines: Vec<String>,
    },
    NonStreaming {
        request_id: String,
        params: MessageSendParams,
        litellm_params: BTreeMap<String, AgentJson>,
        #[serde(default)]
        agent_extra_headers: Option<BTreeMap<String, String>>,
        answer: Answer,
    },
    Streaming {
        request_id: String,
        params: MessageSendParams,
        litellm_params: BTreeMap<String, AgentJson>,
        #[serde(default)]
        agent_extra_headers: Option<BTreeMap<String, String>>,
        answer: StreamAnswer,
    },
    Config {
        request_id: String,
        params: MessageSendParams,
        #[serde(default)]
        litellm_params: Option<BTreeMap<String, AgentJson>>,
        answer: Answer,
        stream_answer: StreamAnswer,
    },
}

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Request(#[from] AgentCoreRequestError),
    #[error("the fixture has no {0} reply")]
    Missing(&'static str),
    #[error("the fixture reply could not be encoded")]
    Encode(#[source] serde_json::Error),
}

struct FixtureRuntime<'case> {
    answer: Option<&'case Answer>,
    stream: Option<&'case StreamAnswer>,
}

impl AgentCoreRuntime for FixtureRuntime<'_> {
    type Error = Failure;
    type Lines = Iter<std::vec::IntoIter<Result<String, Failure>>>;

    fn complete_url(
        &self,
        model: &str,
        optional_params: &BTreeMap<String, AgentJson>,
    ) -> Result<String, Self::Error> {
        complete_url(model, &param_map(optional_params)).map_err(Failure::Request)
    }

    fn session_id(
        &self,
        optional_params: &BTreeMap<String, AgentJson>,
        generated: Uuid,
    ) -> Result<String, Self::Error> {
        runtime_session_id(&param_map(optional_params), generated).map_err(Failure::Request)
    }

    fn user_id(
        &self,
        optional_params: &BTreeMap<String, AgentJson>,
    ) -> Result<Option<String>, Self::Error> {
        runtime_user_id(&param_map(optional_params)).map_err(Failure::Request)
    }

    async fn sign(&self, request: &SigningRequest<'_>) -> Result<SignedRequest, Self::Error> {
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), Some(value.clone())))
            .collect::<Vec<_>>();
        let signed = sign_request(
            SignRequest {
                headers: &headers,
                optional_params: &param_map(request.optional_params),
                request_data: request.request_data,
                api_base: request.api_base,
                model: Some(request.model),
                api_key: request.api_key,
                configured_bearer: None,
                region_inputs: RegionInputs::default(),
                now: UNIX_EPOCH,
            },
            |_settings, _region| {
                std::future::ready(Ok(Credentials {
                    access_key_id: SecretString::from("fixture-access-key"),
                    secret_access_key: SecretString::from("fixture-secret-key"),
                    session_token: None,
                }))
            },
        )
        .await
        .map_err(Failure::Request)?;
        Ok(SignedRequest {
            url: signed.url,
            headers: signed.headers.into_iter().collect(),
            body: signed.body,
        })
    }

    async fn post(&self, _request: &HttpRequest<'_>) -> Result<HttpResponse, Self::Error> {
        let answer = self.answer.ok_or(Failure::Missing("non-streaming"))?;
        let body = serde_json::to_vec(&answer.body).map_err(Failure::Encode)?;
        Ok(HttpResponse {
            status: answer.status,
            body,
        })
    }

    async fn post_stream(
        &self,
        _request: &HttpRequest<'_>,
    ) -> Result<StreamedResponse<Self::Lines>, Self::Error> {
        let answer = self.stream.ok_or(Failure::Missing("streaming"))?;
        let lines: Vec<Result<String, Failure>> = answer.lines.iter().cloned().map(Ok).collect();
        Ok(StreamedResponse {
            status: answer.status,
            content_type: answer.content_type.clone(),
            lines: stream::iter(lines.into_iter()),
        })
    }
}

fn param_value(value: &AgentJson) -> ParamValue {
    match value {
        AgentJson::Null => ParamValue::Null,
        AgentJson::Bool(flag) => ParamValue::Bool(*flag),
        AgentJson::Number(number) => ParamValue::Number(number.clone()),
        AgentJson::String(text) => ParamValue::Text(text.clone()),
        AgentJson::Array(items) => ParamValue::List(items.iter().map(param_value).collect()),
        AgentJson::Object(fields) => ParamValue::Mapping(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), param_value(item)))
                .collect(),
        ),
    }
}

fn param_map(params: &BTreeMap<String, AgentJson>) -> ParamMap {
    params
        .iter()
        .map(|(name, value)| (name.clone(), param_value(value)))
        .collect()
}

struct Source;

impl EventSource for Source {
    fn next_uuid(&mut self) -> Uuid {
        Uuid::from_u128(7)
    }

    fn now(&mut self) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }
}

fn helper_failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn failed(error: &AgentCoreError<Failure>) -> Box<HelperError> {
    let status = match error {
        AgentCoreError::Transform(TransformError::SessionId { .. }) => Some(400),
        AgentCoreError::Transform(_)
        | AgentCoreError::Status { .. }
        | AgentCoreError::Runtime(_) => None,
    };
    Box::new(HelperError {
        class: "AgentCoreError".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn parameters(raw: &BTreeMap<String, AgentJson>) -> Result<LitellmParams, Box<HelperError>> {
    LitellmParams::parse(raw).map_err(|error| helper_failure("TransformError", error.to_string()))
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| helper_failure("RuntimeError", error.to_string()))
}

fn signed(
    request_id: &str,
    params: &MessageSendParams,
    litellm_params: &BTreeMap<String, AgentJson>,
    agent_extra_headers: Option<&BTreeMap<String, String>>,
) -> HelperResult {
    let parameters = parameters(litellm_params)?;
    let fixture = FixtureRuntime {
        answer: None,
        stream: None,
    };
    let result = runtime()?.block_on(get_url_and_signed_request(
        &fixture,
        &SignedRequestCall {
            request_id,
            params,
            litellm_params: &parameters,
            method: RpcMethod::Send,
            agent_extra_headers,
        },
        &mut Source,
    ));
    match result {
        Ok(request) => {
            let body = serde_json::from_slice(&request.body)
                .map_err(|error| helper_failure("SerializationError", error.to_string()))?;
            Ok(serde_json::json!({"url": request.url, "headers": request.headers, "body": body}))
        }
        Err(error) => Err(failed(&error)),
    }
}

fn sse(lines: &[String]) -> HelperResult {
    let items: Vec<Result<String, Failure>> = lines.iter().cloned().map(Ok).collect();
    let result = runtime()?.block_on(async {
        let mut events = std::pin::pin!(parse_sse_events(stream::iter(items.into_iter())));
        let mut values = Vec::new();
        while let Some(item) = events.next().await {
            match item {
                Ok(value) => values.push(value),
                Err(error) => return Err(error),
            }
        }
        Ok(values)
    });
    match result {
        Ok(values) => Ok(serde_json::json!({"events": values})),
        Err(error) => Err(failed(&error)),
    }
}

fn collect<S>(
    runtime: &tokio::runtime::Runtime,
    stream: S,
) -> Result<Vec<AgentJson>, Box<HelperError>>
where
    S: Stream<Item = Result<AgentJson, AgentCoreError<Failure>>>,
{
    let values = runtime.block_on(async {
        let mut stream = std::pin::pin!(stream);
        let mut values = Vec::new();
        while let Some(item) = stream.next().await {
            match item {
                Ok(value) => values.push(value),
                Err(error) => return Err(error),
            }
        }
        Ok(values)
    });
    values.map_err(|error| failed(&error))
}

#[derive(serde::Serialize)]
struct Outcome {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<AgentJson>,
}

#[derive(serde::Serialize)]
struct ConfigOutput {
    non_streaming: Outcome,
    streaming: Outcome,
}

fn config(
    request_id: &str,
    params: &MessageSendParams,
    litellm_params: Option<&BTreeMap<String, AgentJson>>,
    answer: &Answer,
    stream_answer: &StreamAnswer,
) -> HelperResult {
    let runtime = runtime()?;
    let mut non_streaming = BedrockAgentCoreConfig {
        handler: AgentCoreHandler {
            runtime: FixtureRuntime {
                answer: Some(answer),
                stream: None,
            },
            ids: Source,
        },
    };
    let call = ConfigCall {
        request_id,
        params,
        api_base: None,
        litellm_params,
        agent_extra_headers: None,
    };
    let first = match runtime.block_on(non_streaming.handle_non_streaming(&call)) {
        Ok(value) => Outcome {
            ok: true,
            value: Some(value),
        },
        Err(AgentCoreError::Transform(TransformError::MissingParams)) => Outcome {
            ok: false,
            value: None,
        },
        Err(error) => return Err(failed(&error)),
    };
    let mut streaming = BedrockAgentCoreConfig {
        handler: AgentCoreHandler {
            runtime: FixtureRuntime {
                answer: None,
                stream: Some(stream_answer),
            },
            ids: Source,
        },
    };
    let second = match runtime.block_on(streaming.handle_streaming(&call)) {
        Ok(events) => Outcome {
            ok: true,
            value: Some(serde_json::json!(collect(&runtime, events)?)),
        },
        Err(AgentCoreError::Transform(TransformError::MissingParams)) => Outcome {
            ok: false,
            value: None,
        },
        Err(error) => return Err(failed(&error)),
    };
    serde_json::to_value(ConfigOutput {
        non_streaming: first,
        streaming: second,
    })
    .map_err(|error| helper_failure("SerializationError", error.to_string()))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::SignedRequest {
                request_id,
                params,
                litellm_params,
                agent_extra_headers,
            } => signed(
                request_id,
                params,
                litellm_params,
                agent_extra_headers.as_ref(),
            ),
            Input::ParseSse { lines } => sse(lines),
            Input::NonStreaming {
                request_id,
                params,
                litellm_params,
                agent_extra_headers,
                answer,
            } => {
                let parameters = parameters(litellm_params)?;
                let mut handler = AgentCoreHandler {
                    runtime: FixtureRuntime {
                        answer: Some(answer),
                        stream: None,
                    },
                    ids: Source,
                };
                let call = HandlerCall {
                    request_id,
                    params,
                    litellm_params: &parameters,
                    agent_extra_headers: agent_extra_headers.as_ref(),
                };
                match runtime()?.block_on(handler.handle_non_streaming(&call)) {
                    Ok(value) => Ok(value),
                    Err(error) => Err(failed(&error)),
                }
            }
            Input::Streaming {
                request_id,
                params,
                litellm_params,
                agent_extra_headers,
                answer,
            } => {
                let parameters = parameters(litellm_params)?;
                let mut handler = AgentCoreHandler {
                    runtime: FixtureRuntime {
                        answer: None,
                        stream: Some(answer),
                    },
                    ids: Source,
                };
                let call = HandlerCall {
                    request_id,
                    params,
                    litellm_params: &parameters,
                    agent_extra_headers: agent_extra_headers.as_ref(),
                };
                let runtime = runtime()?;
                match runtime.block_on(handler.handle_streaming(&call)) {
                    Ok(events) => Ok(serde_json::json!({"events": collect(&runtime, events)?})),
                    Err(error) => Err(failed(&error)),
                }
            }
            Input::Config {
                request_id,
                params,
                litellm_params,
                answer,
                stream_answer,
            } => config(
                request_id,
                params,
                litellm_params.as_ref(),
                answer,
                stream_answer,
            ),
        }
    }
}
