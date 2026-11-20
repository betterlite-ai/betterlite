use std::error::Error;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::database_faults::DbError as DatabaseFault;

use crate::resolved_key::auth::UserAPIKeyAuth;

// B2317 computes budget_reset_at and B1226 enrolls default teams; both report through this one failure type
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct PortError {
    pub message: String,
    #[source]
    pub cause: Option<Box<dyn Error + Send + Sync>>,
    pub database_fault: Option<DatabaseFault>,
}

impl PortError {
    #[must_use]
    pub fn new(message: &str) -> Self {
        Self {
            message: message.to_owned(),
            cause: None,
            database_fault: None,
        }
    }

    #[must_use]
    pub fn from_error<E>(error: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            message: error.to_string(),
            cause: Some(Box::new(error)),
            database_fault: None,
        }
    }

    #[must_use]
    pub fn with_fault(message: &str, fault: DatabaseFault) -> Self {
        Self {
            message: message.to_owned(),
            cause: None,
            database_fault: Some(fault),
        }
    }
}

// litellm's get_budget_reset_time(budget_duration=...): the reset timestamp a created user starts with
pub trait BudgetResetCalculator: Send + Sync {
    fn reset_time(&self, budget_duration: &str) -> Result<Timestamp, PortError>;
}

// check_if_default_team_set plus add_new_user_to_default_team, with the admin auth the source builds
#[async_trait]
pub trait DefaultTeamEnrollment: Send + Sync {
    async fn enroll(
        &self,
        user_id: &str,
        user_email: Option<&str>,
        auth: &UserAPIKeyAuth,
    ) -> Result<(), PortError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoDefaultTeams;

#[async_trait]
impl DefaultTeamEnrollment for NoDefaultTeams {
    async fn enroll(
        &self,
        _user_id: &str,
        _user_email: Option<&str>,
        _auth: &UserAPIKeyAuth,
    ) -> Result<(), PortError> {
        Ok(())
    }
}
