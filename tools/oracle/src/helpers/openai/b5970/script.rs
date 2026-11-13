use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::error::WatsonxError;
use betterlite_openai::watsonx_auth::transport::WatsonxTransport;
use http::HeaderMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

const DEFAULT_FAILURE_STATUS: u16 = 500;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Route {
    route: String,
    replies: Vec<Reply>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum Reply {
    Failure { failure: bool },
    Answer { status: u16, body: String },
}

pub(super) type Requests = Arc<Mutex<Vec<Value>>>;

pub(super) struct Script {
    replies: Mutex<BTreeMap<String, VecDeque<Reply>>>,
    requests: Requests,
}

impl Script {
    pub(super) fn new(routes: &[Route], requests: Requests) -> Self {
        let replies = routes
            .iter()
            .map(|entry| (entry.route.clone(), entry.replies.iter().cloned().collect()))
            .collect();
        Self {
            replies: Mutex::new(replies),
            requests,
        }
    }

    fn answer(&self, request: &OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let route = format!("{} {}", request.method.as_str(), request.url);
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "method": request.method.as_str(),
                "url": request.url.to_string(),
                "headers": header_json(&request.headers),
                "body": match &request.body {
                    RequestBody::Empty | RequestBody::Multipart { .. } => String::new(),
                    RequestBody::Json(data) | RequestBody::Bytes { data, .. } => {
                        String::from_utf8(data.clone()).unwrap_or_default()
                    }
                },
            }));
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&route)
            .and_then(VecDeque::pop_front);
        match reply {
            Some(Reply::Failure { failure: true }) => Err(TransportError::Closed),
            Some(Reply::Answer { status, body }) => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: body.into_bytes(),
            }),
            Some(Reply::Failure { failure: false }) | None => Ok(UpstreamResponse {
                status: DEFAULT_FAILURE_STATUS,
                headers: HeaderMap::new(),
                body: Vec::new(),
            }),
        }
    }
}

impl WatsonxTransport for Script {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(self.answer(&request))
    }
}

pub(super) fn environment(values: &BTreeMap<String, String>) -> WatsonxEnvironment {
    let find = |name: &str| values.get(name);
    let secret = |name: &str| find(name).map(|value| SecretString::from(value.clone()));
    let text = |name: &str| find(name).cloned();
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

pub(super) fn header_json(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

pub(super) fn error_value(error: &WatsonxError) -> Value {
    match error {
        WatsonxError::Api { status_code, .. } => json!({"error": "watsonx", "status": status_code}),
        WatsonxError::IamApiKeyRequired => json!({"error": "api_key_required"}),
        WatsonxError::InvalidField { field } => json!({"error": "invalid_field", "field": field}),
        WatsonxError::InvalidHeader { header, .. } => {
            json!({"error": "invalid_header", "header": header})
        }
        WatsonxError::IamUrl(_) => json!({"error": "iam_url"}),
        WatsonxError::IamStatus { status, .. } => {
            json!({"error": "iam_status", "status": status})
        }
        WatsonxError::IamTransport(_) => json!({"error": "iam_transport"}),
        WatsonxError::IamReply(_) | WatsonxError::IamReplyField { .. } => {
            json!({"error": "iam_reply"})
        }
        WatsonxError::Cache(_) => json!({"error": "cache"}),
    }
}
