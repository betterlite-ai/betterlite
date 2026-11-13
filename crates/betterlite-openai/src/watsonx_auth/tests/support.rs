use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName};
use secrecy::SecretString;
use serde_json::Value;

use crate::watsonx_auth::WatsonxAuth;
use crate::watsonx_auth::environment::WatsonxEnvironment;
use crate::watsonx_auth::transport::WatsonxTransport;
use crate::watsonx_auth::types::WatsonxParams;

const DEFAULT_FAILURE_STATUS: u16 = 500;

#[derive(Clone)]
pub(super) enum Reply {
    Failure,
    Answer { status: u16, body: String },
}

#[derive(Debug, PartialEq)]
pub(super) struct SentRequest {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) content_type: Option<String>,
    pub(super) accept: Option<String>,
    pub(super) body: String,
}

#[derive(Default)]
pub(super) struct Script {
    replies: Mutex<BTreeMap<String, VecDeque<Reply>>>,
    requests: Mutex<Vec<SentRequest>>,
}

impl Script {
    pub(super) fn token_route(&self, url: &str, replies: Vec<Reply>) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(format!("POST {url}"), VecDeque::from(replies));
    }

    pub(super) fn take(&self) -> Vec<SentRequest> {
        mem::take(&mut *self.requests.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn answer(&self, request: &OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let route = format!("{} {}", request.method.as_str(), request.url);
        let body = match &request.body {
            RequestBody::Empty | RequestBody::Multipart { .. } => String::new(),
            RequestBody::Json(data) | RequestBody::Bytes { data, .. } => {
                String::from_utf8(data.clone()).unwrap_or_default()
            }
        };
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SentRequest {
                method: request.method.as_str().to_owned(),
                url: request.url.to_string(),
                content_type: header(&request.headers, CONTENT_TYPE),
                accept: header(&request.headers, ACCEPT),
                body,
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&route)
            .and_then(VecDeque::pop_front);
        match reply {
            Some(Reply::Failure) => Err(TransportError::Closed),
            Some(Reply::Answer {
                status,
                body: reply_body,
            }) => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: reply_body.into_bytes(),
            }),
            None => Ok(UpstreamResponse {
                status: DEFAULT_FAILURE_STATUS,
                headers: HeaderMap::new(),
                body: Vec::new(),
            }),
        }
    }
}

fn header(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

impl WatsonxTransport for Script {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(self.answer(&request))
    }
}

#[derive(Clone)]
pub(super) struct Clock(Arc<Mutex<f64>>);

impl Clock {
    pub(super) fn new() -> Self {
        Self(Arc::new(Mutex::new(0.0_f64)))
    }

    pub(super) fn read(&self) -> f64 {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn advance(&self, seconds: f64) {
        let mut now = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.mul_add(1.0_f64, seconds);
    }
}

pub(super) fn environment(values: &[(&str, &str)]) -> WatsonxEnvironment {
    let find = |name: &str| {
        values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    };
    let secret = |name: &str| find(name).map(|value| SecretString::from(value.to_owned()));
    let text = |name: &str| find(name).map(str::to_owned);
    WatsonxEnvironment {
        watsonx_iam_url: text("WATSONX_IAM_URL"),
        wx_api_key: secret("WX_API_KEY"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_apikey: secret("WATSONX_APIKEY"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_url: text("WATSONX_URL"),
        wx_url: text("WX_URL"),
        wml_url: text("WML_URL"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        wx_project_id: text("WX_PROJECT_ID"),
        project_id: text("PROJECT_ID"),
        watsonx_region: text("WATSONX_REGION"),
        wx_region: text("WX_REGION"),
        region: text("REGION"),
        watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
        watsonx_space_id: text("WATSONX_SPACE_ID"),
        wx_space_id: text("WX_SPACE_ID"),
        space_id: text("SPACE_ID"),
    }
}

pub(super) fn fixture(
    script: Script,
    clock: &Clock,
    values: &[(&str, &str)],
) -> WatsonxAuth<Script> {
    let shared = Clock::clone(clock);
    WatsonxAuth::new(environment(values), script, Arc::new(move || shared.read())).unwrap()
}

pub(super) fn answer(status: u16, body: &str) -> Reply {
    Reply::Answer {
        status,
        body: body.to_owned(),
    }
}

pub(super) fn token_reply(token: &str, expires_in: i64) -> Reply {
    answer(
        200,
        &format!(r#"{{"access_token":"{token}","expires_in":{expires_in}}}"#),
    )
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn params(entries: &[(&str, Value)]) -> WatsonxParams {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(super) fn object(entries: &[(&str, Value)]) -> Value {
    let fields: BTreeMap<String, Value> = entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect();
    serde_json::to_value(fields).unwrap()
}
