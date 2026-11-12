use thiserror::Error;
#[derive(Debug, PartialEq, Eq, Error)]
pub enum HostedChatOptionsError {
    #[error("Featherless AI does not support the {name} parameter")]
    UnsupportedParam { name: String },
    #[error("Missing Featherless AI API Key")]
    MissingApiKey,
}
impl HostedChatOptionsError {
    // UnsupportedParamsError carries 400; the pinned missing-key check raises a bare ValueError with no status
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::UnsupportedParam { .. } => Some(400),
            Self::MissingApiKey => None,
        }
    }
}
