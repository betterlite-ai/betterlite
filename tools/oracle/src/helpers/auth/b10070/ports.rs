use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;

use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::ui_session_identity::UiSessionTeams;

use super::LookupAnswer;

#[derive(Debug, thiserror::Error)]
#[error("the case user lookup failed")]
pub(super) struct CaseFailure;

// the case's database and user read, with the flags the answer reports back
pub(super) struct CaseTeams<'case> {
    pub(super) database: bool,
    pub(super) lookup: &'case LookupAnswer,
    pub(super) ran: AtomicBool,
    pub(super) fresh: AtomicBool,
}

#[async_trait]
impl UiSessionTeams for CaseTeams<'_> {
    fn has_database(&self) -> bool {
        self.database
    }

    async fn user_teams(
        &self,
        _user_id: &str,
        fresh: bool,
    ) -> Result<Option<Vec<String>>, UserLookupError> {
        self.ran.store(true, Ordering::SeqCst);
        self.fresh.store(fresh, Ordering::SeqCst);
        match self.lookup {
            LookupAnswer::Teams { teams } => Ok(Some(teams.clone())),
            LookupAnswer::NoUser => Ok(None),
            LookupAnswer::Failure => Err(UserLookupError::Unavailable(Box::new(CaseFailure))),
        }
    }
}
