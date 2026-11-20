use std::sync::Arc;

use betterlite_db::postgres_store::errors::DbError;
use thiserror::Error;

use super::rows::MembershipRowError;
use crate::auth_cache::AuthCacheError;
use crate::auth_cache::payload::PayloadError;
use crate::auth_read::table::AuthReadError;

// A membership read through B2315's bounded table; an access or session fault carries no row failure.
#[derive(Debug, Error)]
pub enum MembershipReadFailure {
    // The access or session failed before a table existed, so its repository error stands alone.
    #[error(transparent)]
    Access(Box<DbError>),
    #[error("the team membership lookup failed")]
    Table {
        #[source]
        source: Box<AuthReadError<MembershipRowError>>,
    },
}

// Every way auth_checks.py's get_team_membership can leave before answering a membership.
#[derive(Debug, Error)]
pub enum MembershipLoadError {
    // The source's raise Exception("No db connected") at auth_checks.py L2422; no status, type or code here.
    #[error("No db connected")]
    NoDatabase,
    #[error("the team membership cache read failed")]
    CacheRead(#[source] AuthCacheError),
    #[error("the team membership cache write failed")]
    CacheWrite(#[source] AuthCacheError),
    #[error("the cached team membership could not be decoded")]
    Decode(#[from] PayloadError),
    #[error(transparent)]
    Read(#[from] MembershipReadFailure),
    // A waiter shares the load's one failure, as every caller awaiting the source task sees one exception.
    #[error(transparent)]
    Shared(Arc<Self>),
    #[error("the shared team membership load ended without a result")]
    LoadLost,
}
