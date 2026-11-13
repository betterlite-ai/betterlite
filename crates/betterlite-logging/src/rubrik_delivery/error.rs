use http::header::InvalidHeaderValue;
use thiserror::Error;

use super::transport::TransportFailure;

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::Display)]
pub enum Service {
    #[strum(serialize = "Response moderation service")]
    ResponseModeration,
    #[strum(serialize = "Prompt moderation service")]
    PromptModeration,
    #[strum(serialize = "Logging batch service")]
    Logging,
}

#[derive(Debug, Error)]
pub enum RubrikError {
    #[error("Rubrik webhook URL not configured. Set RUBRIK_WEBHOOK_URL or pass api_base.")]
    MissingWebhookUrl,
    #[error("the Rubrik API key cannot be carried in an Authorization header")]
    InvalidApiKey {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("{service} request failed")]
    Transport {
        service: Service,
        #[source]
        source: TransportFailure,
    },
    #[error("{service} returned HTTP {status}")]
    HttpStatus { service: Service, status: u16 },
    #[error(
        "{service} returned non-object JSON; expected an OpenAI chat completion shape or empty object"
    )]
    NonObjectResponse { service: Service },
    #[error("{service} returned a body that is not JSON")]
    ResponseBody {
        service: Service,
        #[source]
        source: serde_json::Error,
    },
    #[error("the {service} request payload could not be serialized")]
    Serialize {
        service: Service,
        #[source]
        source: serde_json::Error,
    },
}
