use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// ahealth_check's request: the prepared params, the resolved mode and the prompt and input the probe sends
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeRequest {
    pub params: JsonObject,
    pub mode: Option<String>,
    pub prompt: String,
    pub input: Vec<String>,
}

// _create_health_check_response: the rate-limit headers a healthy provider answered, each kept only when it was sent
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProbeResponse {
    pub remaining_requests: Option<String>,
    pub remaining_tokens: Option<String>,
    pub region: Option<String>,
}

impl ProbeResponse {
    #[must_use]
    pub fn to_json(&self) -> JsonObject {
        let mut response = JsonObject::new();
        for (key, value) in [
            ("x-ratelimit-remaining-requests", &self.remaining_requests),
            ("x-ratelimit-remaining-tokens", &self.remaining_tokens),
            ("x-ms-region", &self.region),
        ] {
            if let Some(value) = value {
                response.insert(key.to_owned(), Json::Text(value.clone()));
            }
        }
        response
    }
}

// The Python exception ahealth_check hands back beside its error text: the status a cache reader keeps and the raw request it logged.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ProbeFailure {
    pub message: String,
    pub status_code: u16,
    pub raw_request_typed_dict: Option<Json>,
}

impl ProbeFailure {
    #[must_use]
    pub fn internal(message: String) -> Self {
        Self {
            message,
            status_code: 500,
            raw_request_typed_dict: None,
        }
    }

    #[must_use]
    pub fn to_json(&self) -> JsonObject {
        let mut response = JsonObject::new();
        response.insert("error".to_owned(), Json::Text(self.message.clone()));
        if let Some(raw) = &self.raw_request_typed_dict {
            response.insert("raw_request_typed_dict".to_owned(), raw.clone());
        }
        response
    }
}

// litellm.ahealth_check, injected: the transport answers with the rate-limit headers or with the exception the provider raised.
#[async_trait]
pub trait ProviderHealth: Send + Sync {
    async fn check(&self, request: ProbeRequest) -> Result<ProbeResponse, ProbeFailure>;
}
