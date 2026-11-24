use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshFailureKind {
    Rejected,
    Unavailable,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{detail}")]
pub struct RefreshFailure {
    pub kind: RefreshFailureKind,
    pub detail: String,
}
