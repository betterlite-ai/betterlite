use std::error::Error;
use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::{Instant, sleep};
use tokio_util::task::TaskTracker;
use tracing::{debug, error, info, warn};

use super::health::HealthCheck;
use super::{LifecycleFuture, ProxyStateSink, TrackedTask};
use crate::database_views::contracts::SupportsRawQueries;
use crate::database_views::definitions::{
    DAILY_TAG_SPEND, LAST_30D_KEYS_BY_SPEND, LAST_30D_MODELS_BY_SPEND,
    LAST_30D_TOP_END_USERS_SPEND, MONTHLY_GLOBAL_SPEND, MONTHLY_GLOBAL_SPEND_PER_KEY,
    MONTHLY_USER_KEY_SPEND, VERIFICATION_TOKEN_VIEW,
};
use crate::database_views::estimate::should_create_missing_views;
use crate::database_views::runtime::{create_missing_views, create_view_tolerating_race};

pub const VIEW_SETUP_POLL_INTERVAL: Duration = Duration::from_secs(5);
pub const VIEW_SETUP_DEADLINE: Duration = Duration::from_mins(15);
pub const VIEW_SETUP_GATE_TABLE: &str = "\"LiteLLM_SpendLogs\"";
pub const EXPECTED_VIEWS: [&str; 8] = [
    VERIFICATION_TOKEN_VIEW,
    MONTHLY_GLOBAL_SPEND,
    LAST_30D_KEYS_BY_SPEND,
    LAST_30D_MODELS_BY_SPEND,
    MONTHLY_GLOBAL_SPEND_PER_KEY,
    MONTHLY_USER_KEY_SPEND,
    LAST_30D_TOP_END_USERS_SPEND,
    DAILY_TAG_SPEND,
];
pub const TOKEN_VIEW_REPAIR_SQL: &str = r#"CREATE VIEW "LiteLLM_VerificationTokenView" AS
SELECT v.*, t.spend AS team_spend, t.max_budget AS team_max_budget, t.model_max_budget AS team_model_max_budget, t.tpm_limit AS team_tpm_limit, t.rpm_limit AS team_rpm_limit, t.tpd_limit AS team_tpd_limit
FROM "LiteLLM_VerificationToken" v
LEFT JOIN "LiteLLM_TeamTable" t ON v.team_id = t.team_id;"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewSetupOutcome {
    Ready,
    TimedOut,
}

enum ViewSetupFailure {
    TableMissing,
    Failed(String),
}

pub trait ViewCatalog: SupportsRawQueries + Sync {
    fn gate_table_present(&self) -> LifecycleFuture<'_, Result<bool, Self::Error>>;

    fn existing_views<'borrow>(
        &'borrow self,
        schema: &'borrow str,
        expected: &'borrow [&'borrow str],
    ) -> LifecycleFuture<'borrow, Result<Vec<String>, Self::Error>>;
}

#[derive(Debug, thiserror::Error)]
pub enum ViewsError<DatabaseError, CheckError>
where
    DatabaseError: Error + Send + Sync + 'static,
    CheckError: Error + Send + Sync + 'static,
{
    #[error("the spend view catalog could not be read: {source}")]
    Database {
        #[source]
        source: DatabaseError,
    },
    #[error("the database health check before creating the token view failed: {source}")]
    Health {
        #[source]
        source: CheckError,
    },
}

pub struct ViewSetup<Database, Health> {
    pub database: Database,
    pub health: Health,
    pub sink: Arc<dyn ProxyStateSink>,
    pub schema: String,
    pub poll_interval: Duration,
    pub deadline: Duration,
    pub task: TrackedTask,
}

impl<Database, Health> ViewSetup<Database, Health>
where
    Database: ViewCatalog,
    Health: HealthCheck,
{
    #[must_use]
    pub async fn run(&self) -> ViewSetupOutcome {
        let started = Instant::now();
        loop {
            let Err(failure) = self.attempt().await else {
                return ViewSetupOutcome::Ready;
            };
            if started.elapsed() >= self.deadline {
                self.log_timeout(&failure);
                return ViewSetupOutcome::TimedOut;
            }
            sleep(self.poll_interval).await;
        }
    }

    pub fn start(self: &Arc<Self>, tasks: &TaskTracker)
    where
        Self: Send + Sync + 'static,
    {
        let setup = Arc::clone(self);
        let _started = self.task.start_with(tasks, async move {
            let _outcome = setup.run().await;
        });
    }

    async fn attempt(&self) -> Result<(), ViewSetupFailure> {
        let present = self
            .database
            .gate_table_present()
            .await
            .map_err(failed_attempt)?;
        if !present {
            debug!("Waiting for table {VIEW_SETUP_GATE_TABLE} before creating the spend views");
            return Err(ViewSetupFailure::TableMissing);
        }
        self.health
            .publish_spend_logs_row_count(self.sink.as_ref())
            .await;
        check_view_exists(&self.database, &self.health, &self.schema)
            .await
            .map_err(failed_attempt)
    }

    fn log_timeout(&self, failure: &ViewSetupFailure) {
        match failure {
            ViewSetupFailure::Failed(error) => error!(
                "Gave up creating the spend views after {:?}s; the last attempt failed with: {error}. Fix that error and restart the proxy.",
                self.deadline.as_secs_f64()
            ),
            ViewSetupFailure::TableMissing => error!(
                "Gave up creating the spend views: table {VIEW_SETUP_GATE_TABLE} did not appear within {:?}s. Run the database migrations against this database and restart the proxy.",
                self.deadline.as_secs_f64()
            ),
        }
    }
}

pub async fn check_view_exists<Database, Health>(
    database: &Database,
    health: &Health,
    schema: &str,
) -> Result<(), ViewsError<Database::Error, Health::Error>>
where
    Database: ViewCatalog,
    Health: HealthCheck,
{
    let names = database
        .existing_views(schema, &EXPECTED_VIEWS)
        .await
        .map_err(|source| ViewsError::Database { source })?;
    if names.len() == EXPECTED_VIEWS.len() {
        info!("All necessary views exist!");
        return Ok(());
    }
    let token_view_present = names
        .iter()
        .any(|name| name.as_str() == VERIFICATION_TOKEN_VIEW);
    if !names.is_empty() && !token_view_present {
        return create_token_view(database, health).await;
    }
    if should_create_missing_views(database)
        .await
        .map_err(|source| ViewsError::Database { source })?
    {
        create_missing_views(database)
            .await
            .map_err(|source| ViewsError::Database { source })?;
        return Ok(());
    }
    warn_missing_views(&names);
    Ok(())
}

async fn create_token_view<Database, Health>(
    database: &Database,
    health: &Health,
) -> Result<(), ViewsError<Database::Error, Health::Error>>
where
    Database: ViewCatalog,
    Health: HealthCheck,
{
    health
        .health_check()
        .await
        .map_err(|source| ViewsError::Health { source })?;
    create_view_tolerating_race(database, VERIFICATION_TOKEN_VIEW, TOKEN_VIEW_REPAIR_SQL)
        .await
        .map_err(|source| ViewsError::Database { source })?;
    Ok(())
}

fn warn_missing_views(names: &[String]) {
    let mut missing: Vec<&str> = Vec::new();
    for view in EXPECTED_VIEWS {
        if !names.iter().any(|name| name.as_str() == view) {
            missing.push(view);
        }
    }
    warn!(
        "Not all views exist in db, needed for UI 'Usage' tab. Missing={}. Run 'create_views.py' from https://github.com/BerriAI/litellm/tree/main/db_scripts to create missing views.",
        missing.join(", ")
    );
}

fn failed_attempt<Failure>(failure: Failure) -> ViewSetupFailure
where
    Failure: Display,
{
    warn!("Spend view setup attempt failed, retrying until the schema settles: {failure}");
    ViewSetupFailure::Failed(failure.to_string())
}
