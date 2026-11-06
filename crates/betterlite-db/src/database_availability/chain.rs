use super::classification::{is_permanent_fault, is_service_unavailable};
use crate::database_faults::DbError;

pub const MAX_CHAIN_DEPTH: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncpgFault {
    PostgresConnection,
    Interface,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Database(DbError),
    Os,
    AsyncTimeout,
    Asyncpg(AsyncpgFault),
    AsyncpgUnavailable(AsyncpgFault),
    Other,
    BaseException,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub origin: Origin,
    // str(exception) for every origin, so the stale-plan text check reads a link the same way whatever raised it
    pub message: String,
    pub cause: Option<usize>,
    pub context: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chain {
    pub failures: Vec<Failure>,
}

impl Chain {
    #[must_use]
    pub fn from_error(error: &DbError) -> Self {
        let mut failures: Vec<Failure> = Vec::new();
        let mut current = Some(error);
        let mut previous: Option<usize> = None;
        while let Some(link) = current {
            let index = failures.len();
            if let Some(entry) = previous.and_then(|value| failures.get_mut(value)) {
                entry.cause = Some(index);
            }
            failures.push(Failure {
                origin: Origin::Database(DbError {
                    error_type: link.error_type,
                    message: link.message.clone(),
                    code: link.code.clone(),
                    meta_code: link.meta_code.clone(),
                    traceback_modules: link.traceback_modules.clone(),
                    cause: None,
                }),
                message: link.message.clone(),
                cause: None,
                context: None,
            });
            previous = Some(index);
            current = link.cause.as_deref();
        }
        Self { failures }
    }

    #[must_use]
    pub fn links(&self) -> Vec<&Failure> {
        let mut links = Vec::new();
        let mut current = self.failures.first();
        for _ in 0..MAX_CHAIN_DEPTH {
            let Some(failure) = current else { break };
            links.push(failure);
            current = failure
                .cause
                .or(failure.context)
                .and_then(|index| self.failures.get(index));
        }
        links
    }

    #[must_use]
    pub fn outages(&self) -> Vec<&Failure> {
        let mut outages = Vec::new();
        for failure in self.links() {
            if !matches!(failure.origin, Origin::BaseException) && is_service_unavailable(failure) {
                outages.push(failure);
            }
        }
        outages
    }

    #[must_use]
    pub fn find_outage(&self) -> Option<&Failure> {
        let outages = self.outages();
        let permanent = outages
            .iter()
            .copied()
            .find(|failure| is_permanent_fault(failure));
        permanent.or_else(|| outages.first().copied())
    }
}

#[must_use]
pub fn has_outage_in_chain(chain: &Chain) -> bool {
    chain.links().into_iter().any(|failure| {
        !matches!(failure.origin, Origin::BaseException) && is_service_unavailable(failure)
    })
}
