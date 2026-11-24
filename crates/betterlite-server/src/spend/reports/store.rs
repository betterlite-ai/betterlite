use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::rows::Query;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::repository::{TeamDatabase, TeamQueryError, TeamRepository};
use chrono::{DateTime, Utc};

use crate::organization_create::access::{DatabaseUserLookup, OrgAccessError, OrgUserLookup};

use super::query_service;
use super::rows::{DailyTeamSpend, OrganizationKeySpend, ScopedKeySpend};
use super::scope::ScopeColumn;
use super::sql::{ORGANIZATION_SPEND_REPORT_SQL, scoped_spend_report_sql};

pub type ScopedReportHandle = Option<Arc<dyn ScopedReportStore>>;

#[derive(Debug, thiserror::Error)]
pub enum ScopedReportError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("a spend report row could not be decoded")]
    Decode(#[from] serde_json::Error),
    #[error(transparent)]
    Teams(#[from] TeamQueryError),
}

#[async_trait]
pub trait ScopedReportStore: Send + Sync {
    async fn scoped_report(
        &self,
        column: ScopeColumn,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
        value: &str,
    ) -> Result<Vec<ScopedKeySpend>, ScopedReportError>;
    async fn organization_report(
        &self,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
        organization_id: &str,
        team_ids: &[String],
    ) -> Result<Vec<OrganizationKeySpend>, ScopedReportError>;
    async fn organization_team_ids(
        &self,
        organization_id: &str,
    ) -> Result<Vec<String>, ScopedReportError>;
    async fn find_user(&self, user_id: &str) -> Result<Option<LiteLLMUserTable>, OrgAccessError>;
    async fn spend_by_team(
        &self,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
        team_id: Option<&str>,
    ) -> Result<Vec<DailyTeamSpend>, ScopedReportError>;
}

// B826's org access check reads the caller's memberships through this port
pub(super) struct StoreOrgLookup<'store> {
    pub(super) store: &'store dyn ScopedReportStore,
}

#[async_trait]
impl OrgUserLookup for StoreOrgLookup<'_> {
    async fn find_user(&self, user_id: &str) -> Result<Option<LiteLLMUserTable>, OrgAccessError> {
        self.store.find_user(user_id).await
    }
}

pub struct PostgresScopedReportStore {
    pub store: PostgresStore,
}

#[async_trait]
impl ScopedReportStore for PostgresScopedReportStore {
    async fn scoped_report(
        &self,
        column: ScopeColumn,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
        value: &str,
    ) -> Result<Vec<ScopedKeySpend>, ScopedReportError> {
        let sql = scoped_spend_report_sql(column);
        let query = Query {
            sql: &sql,
            parameters: vec![start, end, &value],
        };
        query_service::decoded_rows(&self.store, &query).await
    }

    async fn organization_report(
        &self,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
        organization_id: &str,
        team_ids: &[String],
    ) -> Result<Vec<OrganizationKeySpend>, ScopedReportError> {
        let teams = team_ids.to_vec();
        let query = Query {
            sql: ORGANIZATION_SPEND_REPORT_SQL,
            parameters: vec![start, end, &organization_id, &teams],
        };
        query_service::decoded_rows(&self.store, &query).await
    }

    async fn organization_team_ids(
        &self,
        organization_id: &str,
    ) -> Result<Vec<String>, ScopedReportError> {
        let teams = TeamRepository {
            database: TeamDatabase {
                db: &self.store,
                writer_db: &self.store,
            },
            use_writer: false,
        }
        .find_by_organization_id(organization_id)
        .await?;
        Ok(teams
            .into_iter()
            .map(|team| team.fields.base.team_id)
            .collect())
    }

    async fn find_user(&self, user_id: &str) -> Result<Option<LiteLLMUserTable>, OrgAccessError> {
        DatabaseUserLookup { store: &self.store }
            .find_user(user_id)
            .await
    }

    async fn spend_by_team(
        &self,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
        team_id: Option<&str>,
    ) -> Result<Vec<DailyTeamSpend>, ScopedReportError> {
        query_service::get_spend_by_team(&self.store, start, end, team_id).await
    }
}
