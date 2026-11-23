mod tests;

use std::fmt;
use std::future::Future;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectIdentity {
    pub user_id: String,
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalCode {
    UnsupportedGrantType,
    InvalidRequest,
    TemporarilyUnavailable,
}

impl RefusalCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedGrantType => "unsupported_grant_type",
            Self::InvalidRequest => "invalid_request",
            Self::TemporarilyUnavailable => "temporarily_unavailable",
        }
    }
}

impl fmt::Display for RefusalCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectTokenRefusal {
    pub error: RefusalCode,
    pub description: String,
}

pub trait ExchangeSubjectToken<Context> {
    fn exchange(
        &self,
        subject_token: &str,
        context: &Context,
    ) -> impl Future<Output = Result<SubjectIdentity, SubjectTokenRefusal>> + Send;
}
