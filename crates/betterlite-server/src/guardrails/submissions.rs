pub mod error;
pub mod json_field;
pub mod models;
pub mod postgres_rows;
pub mod route;
pub mod rows;
pub mod teams;

mod listing;
mod register;
mod tests;

use std::sync::Arc;

use chrono::{DateTime, Utc};

use self::rows::SubmissionRows;
use crate::guardrails::admin_reads::state::UserTeams;

pub struct SubmissionsState<S: SubmissionRows> {
    pub database: Option<Arc<S>>,
    pub users: Arc<dyn UserTeams>,
    pub clock: Arc<dyn Clock>,
}

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
