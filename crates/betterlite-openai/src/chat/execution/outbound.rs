use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{ACCEPT, AUTHORIZATION};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use secrecy::ExposeSecret as _;
use serde_json::{Map, Value};

use super::KNOWN_PARAMS;
use super::{ChatError, ChatExecutor};
use crate::chat::ChatRequest;
use crate::chat::deployment::ChatDeployment;
use crate::openai_base_config::{ConfigFamily, Headers, OpenAiConfig, OpenAiSettings};

impl ChatExecutor {
    pub(super) fn client_for(&self, deployment: &ChatDeployment) -> &ProxiedClient {
        self.client
            .deployments
            .get(&deployment.model_id)
            .unwrap_or(&self.client)
    }

    pub(super) fn body(
        &self,
        request: &ChatRequest,
        model: &str,
    ) -> Result<Map<String, Value>, ChatError> {
        let Value::Object(mut requested) =
            serde_json::to_value(request).map_err(|source| ChatError::Encode { source })?
        else {
            return Ok(Map::new());
        };
        let messages = requested
            .remove("messages")
            .unwrap_or(Value::Array(Vec::new()));
        // the mock control fields are read here and never reach the provider
        for name in ["model", "stream", "mock_response", "mock_tool_calls"] {
            requested.remove(name);
        }
        let policy = OpenAiConfig {
            family: ConfigFamily::OpenAi,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        };
        let optional = self.optional_params(request, model, requested, &policy);
        Ok(policy.request_body(model, messages, &optional))
    }

    fn optional_params(
        &self,
        request: &ChatRequest,
        model: &str,
        requested: Map<String, Value>,
        policy: &OpenAiConfig,
    ) -> Map<String, Value> {
        let (known, forwarded): (Map<String, Value>, Map<String, Value>) = requested
            .into_iter()
            .partition(|(name, _)| KNOWN_PARAMS.split(' ').any(|known| known == name));
        let mut optional = Map::new();
        policy.map_params(model, &known, &mut optional, &self.catalog);
        if request.stream {
            optional.insert(String::from("stream"), Value::Bool(true));
        }
        if request.stream && !optional.contains_key("stream_options") {
            let mut stream_options = Map::new();
            stream_options.insert(String::from("include_usage"), Value::Bool(true));
            optional.insert(
                String::from("stream_options"),
                Value::Object(stream_options),
            );
        }
        optional.extend(forwarded);
        optional
    }
}

pub(super) fn outbound_with(
    deployment: &ChatDeployment,
    accept: &'static str,
    body: &Map<String, Value>,
) -> Result<OutboundRequest, ChatError> {
    let body = serde_json::to_vec(body).map_err(|source| ChatError::Encode { source })?;
    let mut headers = HeaderMap::new();
    let authorization = deployment.credential().ok_or(ChatError::MissingApiKey)?;
    headers.insert(
        AUTHORIZATION,
        header_value(authorization.expose_secret(), "authorization")?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static(accept));
    if let Some(organization) = &deployment.organization {
        headers.insert(
            HeaderName::from_static("openai-organization"),
            header_value(organization, "openai-organization")?,
        );
    }
    Ok(OutboundRequest {
        method: Method::POST,
        url: deployment.endpoint.clone(),
        query: Vec::new(),
        headers,
        body: RequestBody::Json(body),
        deadline: deployment.timeout,
    })
}

fn header_value(text: &str, name: &str) -> Result<HeaderValue, ChatError> {
    HeaderValue::try_from(text).map_err(|source| ChatError::Http {
        source: TransportError::InvalidHeader {
            name: name.to_owned(),
            source,
        },
    })
}
