use std::iter;

use serde::{Deserialize, Serialize};

pub const MAX_CAUSE_DEPTH: usize = 20;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Connection,
    Timeout,
    BusyLoading,
    ClusterDown,
    Os,
    BuiltinTimeout,
    AsynchronousTimeout,
    Command,
    Other,
    CircuitOpen,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Timeout,
    Connectivity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Failure<'causes> {
    pub class: FailureClass,
    pub causes: &'causes [FailureClass],
}

impl FailureKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Connectivity => "connectivity",
        }
    }
}

#[must_use]
pub const fn is_health_failure(class: FailureClass) -> bool {
    matches!(
        class,
        FailureClass::Connection
            | FailureClass::Timeout
            | FailureClass::BusyLoading
            | FailureClass::ClusterDown
            | FailureClass::Os
            | FailureClass::BuiltinTimeout
            | FailureClass::AsynchronousTimeout
    )
}

const fn is_timeout_class(class: FailureClass) -> bool {
    matches!(
        class,
        FailureClass::Timeout | FailureClass::BuiltinTimeout | FailureClass::AsynchronousTimeout
    )
}

#[must_use]
pub fn is_timeout_failure(failure: FailureClass, causes: &[FailureClass]) -> bool {
    iter::once(failure)
        .chain(causes.iter().copied())
        .take(MAX_CAUSE_DEPTH)
        .any(is_timeout_class)
}

#[must_use]
pub fn failure_kind(failure: FailureClass, causes: &[FailureClass]) -> Option<FailureKind> {
    if !is_health_failure(failure) {
        return None;
    }
    Some(if is_timeout_failure(failure, causes) {
        FailureKind::Timeout
    } else {
        FailureKind::Connectivity
    })
}
