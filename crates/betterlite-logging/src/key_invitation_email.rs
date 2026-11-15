pub mod link;
pub mod templates;

mod sending;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::user_repository::RowDecodeError;

use crate::alert_dispatch::destination::AlertDestination;
use crate::budget_email::sender::EmailSender;
use crate::smtp_send::SendEmailError;

#[derive(Debug, thiserror::Error)]
pub enum KeyInvitationEmailError {
    #[error("the database query failed")]
    Database {
        #[from]
        source: DbError,
    },
    #[error("the recipient user row cannot be read")]
    Recipient {
        #[from]
        source: DbError<RowDecodeError>,
    },
    #[error("the team lookup failed")]
    Team {
        #[from]
        source: TeamQueryError,
    },
    #[error("the email could not be sent")]
    Send {
        #[from]
        source: SendEmailError,
    },
}

pub struct KeyInvitationEmailer<'deps, C> {
    pub alerting: Option<&'deps [AlertDestination]>,
    pub environment: &'deps (dyn EnvironmentLookup + Send + Sync),
    pub database: Option<&'deps PostgresStore>,
    pub sender: &'deps dyn EmailSender,
    pub invitation_creator: &'deps C,
}

mod tests;
