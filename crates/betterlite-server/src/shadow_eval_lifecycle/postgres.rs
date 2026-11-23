pub mod rows;
pub mod sql;

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::loader::startup::process_environment;
use betterlite_db::database_tls::options::native_connection_options;
use betterlite_db::postgres_store::errors::{DbError, backend, is_unique_violation};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::OidcTokenCache;
use chrono::{DateTime, NaiveDateTime, Utc};
use deadpool_postgres::{Manager, Pool};
use secrecy::ExposeSecret as _;
use thiserror::Error;

use self::rows::{attempt_row, claimed_row, key_row, leg_parameters, leg_row, team_row, user_row};
use self::sql::{
    ATTEMPT_COUNTS_SQL, CLAIMED_SQL, KEYS_SQL, LEGS_SQL, SEED_FUNNEL_SQL, STOP_JOB_SQL,
    SWEEP_FINISHED_SQL, TEAMS_SQL, USERS_SQL, insert_legs,
};
use super::store::{
    ClaimedTarget, InsertRejection, LegAttemptStats, LegRow, NewLeg, ShadowLifecycleStore,
    StoreDecodeError, TargetRows, target_type_text,
};
use crate::jobs::ptu::attribution::NoOidc;
use crate::shadow_eval_job_loading::jobs::{ShadowEvalDirection, ShadowJobError};
use crate::shadow_eval_models::ShadowEvalTargetType;
use crate::state::ServerState;

// the rows a shadow eval write touches, over the same pool builder the cache activity routes use
pub struct PostgresShadowLifecycleStore {
    pub(super) store: PostgresStore,
}

// a read or write that failed: the driver's error, or a stored value none of the typed columns accept
#[derive(Debug, Error)]
pub enum PostgresLifecycleError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("the stored shadow eval direction could not be read")]
    Direction(#[from] ShadowJobError),
    #[error("the stored shadow eval target type could not be read")]
    TargetType(#[from] StoreDecodeError),
}

// the startup wiring the route needs: the writer URL the CLI resolved, or none while no database is configured
#[must_use]
pub fn store_from_state(state: &Arc<ServerState>) -> Option<PostgresShadowLifecycleStore> {
    let secrets = SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    };
    let resolved = state
        .config
        .sections
        .database_urls
        .cli_writer_url(&process_environment(), &secrets);
    let url = match resolved {
        Ok(Some(url)) => url,
        Ok(None) => return None,
        Err(error) => {
            tracing::error!(%error, "the configured database URL cannot be resolved");
            return None;
        }
    };
    let options = match native_connection_options(url.expose_secret()) {
        Ok(options) => options,
        Err(error) => {
            tracing::error!(%error, "the database URL cannot open the shadow eval pool");
            return None;
        }
    };
    let manager = Manager::new(options.config, options.tls);
    match Pool::builder(manager).build() {
        Ok(pool) => Some(PostgresShadowLifecycleStore {
            store: PostgresStore { pool: Some(pool) },
        }),
        Err(error) => {
            tracing::error!(%error, "the shadow eval pool cannot be built");
            None
        }
    }
}

#[async_trait]
impl ShadowLifecycleStore for PostgresShadowLifecycleStore {
    type Error = PostgresLifecycleError;

    async fn load_targets(
        &self,
        keys: &[String],
        teams: &[String],
        users: &[String],
    ) -> Result<TargetRows, Self::Error> {
        let client = self.store.session().await?;
        let key_ids = keys.to_vec();
        let team_ids = teams.to_vec();
        let user_ids = users.to_vec();
        let mut rows = TargetRows::default();
        let key_rows = client.query(KEYS_SQL, &[&key_ids]).await.map_err(backend)?;
        rows.keys = key_rows.iter().map(key_row).collect::<Result<_, _>>()?;
        let team_rows = client
            .query(TEAMS_SQL, &[&team_ids])
            .await
            .map_err(backend)?;
        rows.teams = team_rows.iter().map(team_row).collect::<Result<_, _>>()?;
        let user_rows = client
            .query(USERS_SQL, &[&user_ids])
            .await
            .map_err(backend)?;
        rows.users = user_rows.iter().map(user_row).collect::<Result<_, _>>()?;
        Ok(rows)
    }

    async fn sweep_finished(
        &self,
        ids: &[String],
        target_type: ShadowEvalTargetType,
    ) -> Result<(), Self::Error> {
        let client = self.store.session().await?;
        let targets = ids.to_vec();
        let kind = target_type_text(target_type);
        client
            .execute(SWEEP_FINISHED_SQL, &[&targets, &kind])
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn claimed_targets(
        &self,
        targets: &[(ShadowEvalTargetType, Vec<String>)],
        direction: ShadowEvalDirection,
    ) -> Result<Vec<ClaimedTarget>, Self::Error> {
        let client = self.store.session().await?;
        let mut keys: Vec<String> = Vec::new();
        let mut teams: Vec<String> = Vec::new();
        let mut users: Vec<String> = Vec::new();
        for (target_type, ids) in targets {
            match target_type {
                ShadowEvalTargetType::Key => keys.clone_from(ids),
                ShadowEvalTargetType::Team => teams.clone_from(ids),
                ShadowEvalTargetType::User => users.clone_from(ids),
            }
        }
        let heading = direction.as_str();
        let rows = client
            .query(CLAIMED_SQL, &[&heading, &keys, &teams, &users])
            .await
            .map_err(backend)?;
        rows.iter().map(claimed_row).collect()
    }

    async fn create_legs(&self, legs: &[NewLeg]) -> Result<(), InsertRejection<Self::Error>> {
        let client = match self.store.session().await {
            Ok(client) => client,
            Err(error) => {
                return Err(InsertRejection::Database(PostgresLifecycleError::Database(
                    error,
                )));
            }
        };
        let created: Vec<NaiveDateTime> =
            legs.iter().map(|leg| leg.created_at.naive_utc()).collect();
        let ends: Vec<NaiveDateTime> = legs.iter().map(|leg| leg.ends_at.naive_utc()).collect();
        let kinds: Vec<&str> = legs
            .iter()
            .map(|leg| target_type_text(leg.target_type))
            .collect();
        let headings: Vec<&str> = legs.iter().map(|leg| leg.direction.as_str()).collect();
        let parameters = leg_parameters(legs, &created, &ends, &kinds, &headings);
        let statement = insert_legs(legs.len());
        match client.execute(&statement, &parameters).await {
            Ok(_written) => Ok(()),
            Err(driver) => {
                let error = backend(driver);
                if is_unique_violation(&error) {
                    return Err(InsertRejection::UniqueViolation);
                }
                Err(InsertRejection::Database(PostgresLifecycleError::Database(
                    error,
                )))
            }
        }
    }

    async fn seed_funnel(&self, job_ids: &[String]) -> Result<(), Self::Error> {
        let client = self.store.session().await?;
        let ids = job_ids.to_vec();
        let executed = client.execute(SEED_FUNNEL_SQL, &[&ids]).await;
        executed.map_err(backend)?;
        Ok(())
    }

    async fn legs(&self, group_id: &str) -> Result<Vec<LegRow>, Self::Error> {
        let client = self.store.session().await?;
        let rows = client
            .query(LEGS_SQL, &[&group_id])
            .await
            .map_err(backend)?;
        rows.iter().map(leg_row).collect()
    }

    async fn attempt_stats(&self, job_ids: &[String]) -> Result<Vec<LegAttemptStats>, Self::Error> {
        let client = self.store.session().await?;
        let ids = job_ids.to_vec();
        let rows = client
            .query(ATTEMPT_COUNTS_SQL, &[&ids])
            .await
            .map_err(backend)?;
        rows.iter().map(attempt_row).collect()
    }

    async fn stop_job(
        &self,
        job_id: &str,
        operator: &str,
        stamp: DateTime<Utc>,
    ) -> Result<u64, Self::Error> {
        let client = self.store.session().await?;
        let naive = stamp.naive_utc();
        client
            .execute(STOP_JOB_SQL, &[&job_id, &operator, &naive])
            .await
            .map_err(backend)
    }
}
