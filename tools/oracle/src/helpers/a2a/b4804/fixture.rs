use std::collections::BTreeMap;
use std::time::UNIX_EPOCH;

use betterlite_a2a::agentcore_transform::SignedRequest;
use betterlite_a2a::agentcore_transform::call::AgentCoreHandler;
use betterlite_a2a::agentcore_transform::runtime::{
    AgentCoreRuntime, HttpRequest, HttpResponse, SigningRequest, StreamedResponse,
};
use betterlite_a2a::completion_bridge::EventSource;
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
use secrecy::SecretString;
use serde::Deserialize;
use uuid::Uuid;

type Params = BTreeMap<String, AgentJson>;

#[derive(Clone, Deserialize)]
pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) body: AgentJson,
}

#[derive(Clone, Deserialize)]
pub(super) struct StreamAnswer {
    pub(super) status: u16,
    pub(super) content_type: String,
    pub(super) lines: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Failure {
    #[error(transparent)]
    Request(#[from] AgentCoreRequestError),
    #[error("the fixture has no {0} reply")]
    Missing(&'static str),
    #[error("the fixture reply could not be encoded")]
    Encode(#[source] serde_json::Error),
}

pub(super) struct FixtureRuntime<'case> {
    answer: Option<&'case Answer>,
    stream: Option<&'case StreamAnswer>,
}

impl<'case> FixtureRuntime<'case> {
    pub(super) const fn silent() -> Self {
        Self {
            answer: None,
            stream: None,
        }
    }

    pub(super) const fn buffered(answer: &'case Answer) -> Self {
        Self {
            answer: Some(answer),
            stream: None,
        }
    }

    pub(super) const fn streamed(stream: &'case StreamAnswer) -> Self {
        Self {
            answer: None,
            stream: Some(stream),
        }
    }

    pub(super) const fn into_handler(self) -> AgentCoreHandler<Self, Source> {
        AgentCoreHandler {
            runtime: self,
            ids: Source,
        }
    }
}

impl AgentCoreRuntime for FixtureRuntime<'_> {
    type Error = Failure;
    type Lines = Iter<std::vec::IntoIter<Result<String, Failure>>>;

    fn complete_url(&self, model: &str, optional_params: &Params) -> Result<String, Self::Error> {
        complete_url(model, &param_map(optional_params)).map_err(Failure::Request)
    }

    fn session_id(&self, optional_params: &Params, generated: Uuid) -> Result<String, Self::Error> {
        runtime_session_id(&param_map(optional_params), generated).map_err(Failure::Request)
    }

    fn user_id(&self, optional_params: &Params) -> Result<Option<String>, Self::Error> {
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
        Ok(HttpResponse {
            status: answer.status,
            body: serde_json::to_vec(&answer.body).map_err(Failure::Encode)?,
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
            lines: stream::iter(lines),
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

pub(super) struct Source;

impl EventSource for Source {
    fn next_uuid(&mut self) -> Uuid {
        Uuid::from_u128(7)
    }

    fn now(&mut self) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }
}
