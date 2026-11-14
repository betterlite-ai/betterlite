use std::time::{SystemTimeError, TryFromFloatSecsError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_env::EnvError;
use indexmap::IndexMap;
use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_records::error::GuardrailRecordError;
use crate::noma_scan_policy::BLOCKED_MESSAGE;
use crate::noma_scan_policy::error::NomaBlockedMessage;

#[derive(Debug, Error)]
pub enum NomaV2Error {
    #[error("Noma v2 guardrail requires api_key when using Noma SaaS endpoint")]
    MissingApiKey,
    #[error("the `{field}` parameter of the Noma v2 guardrail has the wrong type")]
    Param { field: &'static str },
    #[error(transparent)]
    Environment(#[from] EnvError),
    #[error("the Noma v2 timeout {seconds} is not a representable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the Noma endpoint `{api_base}` is not a URL")]
    Endpoint {
        api_base: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the Noma v2 Authorization header is invalid")]
    Header {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Noma API could not be reached")]
    Transport {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Noma API returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Noma API response is not valid JSON")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Noma v2 response has no valid action")]
    InvalidAction,
    #[error("the `{field}` field of the Noma v2 intervention has the wrong shape")]
    Intervention { field: &'static str },
    #[error("the Noma v2 guardrail record could not be written")]
    Record {
        #[source]
        source: GuardrailRecordError,
    },
    #[error("the system clock is set before the Unix epoch")]
    Clock {
        #[source]
        source: SystemTimeError,
    },
    #[error("the Noma v2 guardrail base could not be built")]
    Base {
        #[source]
        source: GuardrailBaseError,
    },
}

impl From<GuardrailBaseError> for NomaV2Error {
    fn from(source: GuardrailBaseError) -> Self {
        Self::Base { source }
    }
}

impl NomaV2Error {
    #[must_use]
    pub fn into_failure(self) -> GuardrailFailure {
        GuardrailFailure::Other(self.to_string())
    }
}

// LiteLLM's NomaBlockedMessage: HTTPException(400, detail={"error": ..., "details": response})
#[must_use]
pub fn blocked_failure(message: &NomaBlockedMessage) -> GuardrailFailure {
    let mut detail = IndexMap::new();
    detail.insert(
        "error".to_owned(),
        GuardrailValue::Text(BLOCKED_MESSAGE.to_owned()),
    );
    detail.insert("details".to_owned(), message.details.clone());
    GuardrailFailure::HttpObject {
        status: NomaBlockedMessage::STATUS_CODE.as_u16(),
        detail,
    }
}
