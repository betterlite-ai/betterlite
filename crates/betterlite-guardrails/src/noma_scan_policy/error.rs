use std::collections::BTreeMap;
use std::time::SystemTimeError;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::response_chat_history::error::HistoryError;
use betterlite_http::user_url::error::UrlParseError;
use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_records::error::GuardrailRecordError;

use super::BLOCKED_MESSAGE;

// LiteLLM's NomaBlockedMessage: HTTPException(400, detail={"error": ..., "details": classification})
#[derive(Debug, Clone, PartialEq, Error)]
#[error("Request blocked by Noma guardrail")]
pub struct NomaBlockedMessage {
    pub details: GuardrailValue,
}

impl NomaBlockedMessage {
    pub const STATUS_CODE: StatusCode = StatusCode::BAD_REQUEST;

    #[must_use]
    pub fn detail(&self) -> GuardrailValue {
        let mut detail = BTreeMap::new();
        detail.insert(
            "error".to_owned(),
            GuardrailValue::Text(BLOCKED_MESSAGE.to_owned()),
        );
        detail.insert("details".to_owned(), self.details.clone());
        GuardrailValue::Object(detail)
    }
}

#[derive(Debug, Error)]
pub enum NomaError {
    #[error("Request blocked by Noma guardrail")]
    Blocked(#[from] NomaBlockedMessage),
    #[error("the `{field}` field of the Noma scan request has the wrong type")]
    Field { field: &'static str },
    #[error("a Noma scan request message is not a chat message")]
    Message {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Noma messages could not be converted to Responses API input")]
    History {
        #[source]
        source: HistoryError,
    },
    #[error("the Noma request payload is not JSON")]
    PayloadJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Noma endpoint `{api_base}` is not a URL")]
    Endpoint {
        api_base: String,
        #[source]
        source: UrlParseError,
    },
    #[error("the `{name}` header the Noma guardrail sends is invalid")]
    Header {
        name: &'static str,
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
    #[error("the Noma API response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Noma API response is not valid JSON")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Noma API response does not have the shape a scan answer needs")]
    ResponseShape {
        #[source]
        source: serde_json::Error,
    },
    #[error("the `{field}` field of the Noma API response has the wrong type")]
    ResponseField { field: &'static str },
    #[error("the Noma guardrail record could not be written")]
    Record {
        #[source]
        source: GuardrailRecordError,
    },
    #[error("the Noma guardrail record is not JSON")]
    RecordJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Noma guardrail base could not be built")]
    RecordBase {
        #[source]
        source: GuardrailBaseError,
    },
    #[error("the system clock is set before the Unix epoch")]
    Clock {
        #[source]
        source: SystemTimeError,
    },
}

impl NomaError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        if matches!(self, Self::Blocked(_)) {
            NomaBlockedMessage::STATUS_CODE
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
