use betterlite_models::identity::provider_id::ProviderId;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResponseInputError {
    #[error(
        "invalid type for 'tools[{index}].parameters': expected an object, but got {found} instead"
    )]
    InvalidToolParameters {
        model: String,
        provider: ProviderId,
        index: usize,
        found: &'static str,
    },
    #[error("'tools' must be a list of tool records, but got {found} instead")]
    ToolsNotAList { found: &'static str },
    #[error(
        "the Responses request already fixes '{key}', so the optional parameters cannot set it"
    )]
    DuplicateBodyKey { key: &'static str },
}

impl ResponseInputError {
    #[must_use]
    pub const fn http_status(&self) -> Option<u16> {
        match self {
            Self::InvalidToolParameters { .. } | Self::ToolsNotAList { .. } => Some(400),
            Self::DuplicateBodyKey { .. } => None,
        }
    }
}
