use crate::otel::config::destination::OtelDestination;
use std::fmt::Debug;
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestOtelScope {
    pub destinations: Vec<OtelDestination>,
    pub additive: bool,
}

impl RequestOtelScope {
    #[must_use]
    pub fn is_suppressed(&self, callback_name: &str) -> bool {
        !self.additive
            && self.destinations.iter().any(|destination| {
                destination.as_ref().callback_name.as_deref() == Some(callback_name)
            })
    }
}

pub trait RequestScopeSource: Send + Sync + Debug {
    fn scope(&self) -> RequestOtelScope;
}

#[path = "scope/tests.rs"]
mod tests;
