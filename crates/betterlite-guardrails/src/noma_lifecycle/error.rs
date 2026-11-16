use std::time::{SystemTimeError, TryFromFloatSecsError};

use betterlite_env::EnvError;
use reqwest::StatusCode;
use thiserror::Error;

use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_records::error::GuardrailRecordError;
use crate::guardrail_selection::error::GuardrailSelectionError;
use crate::noma_scan_policy::error::NomaError;
use crate::noma_v2::error::NomaV2Error;
use crate::third_party_init_a::error::ThirdPartyInitAError;

#[derive(Debug, Error)]
pub enum NomaLifecycleError {
    #[error(transparent)]
    Noma(#[from] NomaError),
    #[error(transparent)]
    Selection(#[from] GuardrailSelectionError),
    #[error(transparent)]
    NomaV2(#[from] NomaV2Error),
    #[error(transparent)]
    Environment(#[from] EnvError),
    #[error(transparent)]
    Params(#[from] ThirdPartyInitAError),
    #[error("the Noma timeout {seconds} is not a representable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the Noma guardrail base could not be built")]
    Base {
        #[source]
        source: GuardrailBaseError,
    },
    #[error("the Noma HTTP client could not be built")]
    Client {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Noma guardrail record could not be written")]
    Record {
        #[source]
        source: GuardrailRecordError,
    },
    #[error("the system clock is set before the Unix epoch")]
    Clock {
        #[source]
        source: SystemTimeError,
    },
}

impl NomaLifecycleError {
    // A block is a 400 and a selection error follows LiteLLM's HTTPException; everything else escapes as a 500
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Noma(error) => error.status_code(),
            Self::Selection(
                GuardrailSelectionError::RequestedNotList
                | GuardrailSelectionError::DynamicParamsNotObject { .. },
            ) => StatusCode::BAD_REQUEST,
            Self::Selection(GuardrailSelectionError::TagMode { .. })
            | Self::NomaV2(_)
            | Self::Environment(_)
            | Self::Params(_)
            | Self::Timeout { .. }
            | Self::Base { .. }
            | Self::Client { .. }
            | Self::Record { .. }
            | Self::Clock { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
