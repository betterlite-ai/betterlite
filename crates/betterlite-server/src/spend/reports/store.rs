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
use super::sql::{ORGANIZATION_SPEND_REPORT_SQL, SCOPED_REPORT_SQL_TEMPLATE, scope_column_sql};

pub type ScopedReportHandle = Option<Arc<dyn ScopedReportStore>>;

// the scope column, window and value a scoped report query binds as $1-$3
#[derive(Clone, Copy)]
pub struct ScopedReportRequest<'request> {
    pub column: ScopeColumn,
    pub start: &'request DateTime<Utc>,
    pub end: &'request DateTime<Utc>,
    pub value: &'request str,
}

// the organization and its current teams, bound as $1-$4
#[derive(Clone, Copy)]
pub struct OrganizationReportRequest<'request> {
    pub start: &'request DateTime<Utc>,
    pub end: &'request DateTime<Utc>,
    pub organization_id: &'request str,
    pub team_ids: &'request [String],
}

// the optional team filter of the day-by-team query, bound as $1-$3
#[derive(Clone, Copy)]
pub struct TeamSpendRequest<'request> {
    pub start: &'request DateTime<Utc>,
    pub end: &'request DateTime<Utc>,
    pub team_id: Option<&'request str>,
}

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
        request: ScopedReportRequest<'_>,
    ) -> Result<Vec<ScopedKeySpend>, ScopedReportError>;
    async fn organization_report(
        &self,
        request: OrganizationReportRequest<'_>,
    ) -> Result<Vec<OrganizationKeySpend>, ScopedReportError>;
    async fn organization_team_ids(
        &self,
        organization_id: &str,
    ) -> Result<Vec<String>, ScopedReportError>;
    async fn find_user(&self, user_id: &str) -> Result<Option<LiteLLMUserTable>, OrgAccessError>;
    async fn spend_by_team(
        &self,
        request: TeamSpendRequest<'_>,
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
        request: ScopedReportRequest<'_>,
    ) -> Result<Vec<ScopedKeySpend>, ScopedReportError> {
        let sql =
            SCOPED_REPORT_SQL_TEMPLATE.replace("{scope_column}", scope_column_sql(request.column));
        let query = Query {
            sql: &sql,
            parameters: vec![request.start, request.end, &request.value],
        };
        query_service::decoded_rows(&self.store, &query).await
    }

    async fn organization_report(
        &self,
        request: OrganizationReportRequest<'_>,
    ) -> Result<Vec<OrganizationKeySpend>, ScopedReportError> {
        organization_rows(&self.store, request).await
    }

    async fn organization_team_ids(
        &self,
        organization_id: &str,
    ) -> Result<Vec<String>, ScopedReportError> {
        let repository = TeamRepository {
            database: TeamDatabase {
                db: &self.store,
                writer_db: &self.store,
            },
            use_writer: false,
        };
        let members = repository.find_by_organization_id(organization_id).await?;
        let mut team_ids = Vec::with_capacity(members.len());
        for team in members {
            team_ids.push(team.fields.base.team_id);
        }
        Ok(team_ids)
    }

    async fn find_user(&self, user_id: &str) -> Result<Option<LiteLLMUserTable>, OrgAccessError> {
        DatabaseUserLookup { store: &self.store }
            .find_user(user_id)
            .await
    }

    async fn spend_by_team(
        &self,
        request: TeamSpendRequest<'_>,
    ) -> Result<Vec<DailyTeamSpend>, ScopedReportError> {
        query_service::get_spend_by_team(&self.store, request.start, request.end, request.team_id)
            .await
    }
}

// _ORG_SPEND_REPORT_SQL over the window, organization and its current teams, decoded into organization rows
async fn organization_rows(
    store: &PostgresStore,
    request: OrganizationReportRequest<'_>,
) -> Result<Vec<OrganizationKeySpend>, ScopedReportError> {
    let query = Query {
        sql: ORGANIZATION_SPEND_REPORT_SQL,
        parameters: vec![
            request.start,
            request.end,
            &request.organization_id,
            &request.team_ids,
        ],
    };
    query_service::decoded_rows(store, &query).await
}
