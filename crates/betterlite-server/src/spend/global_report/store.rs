use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::postgres_store::rows::Query;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, Utc};

use crate::spend::reports::query_service::{decoded_rows, get_spend_by_team};
use crate::spend::reports::scope::ScopeColumn;
use crate::spend::reports::store::{
    PostgresScopedReportStore, ScopedReportError, ScopedReportRequest, ScopedReportStore as _,
};

use super::rows::{DailyTeamCustomerSpend, GlobalReport};
use super::sql::{ALL_KEYS_SQL, CUSTOMER_SQL, TEAM_CUSTOMER_SQL};

pub type GlobalReportHandle = Option<Arc<dyn GlobalReportStore>>;

// the UTC midnights every global report query binds as $1 and $2
#[derive(Clone, Copy)]
pub struct ReportWindow<'window> {
    pub start: &'window DateTime<Utc>,
    pub end: &'window DateTime<Utc>,
}

// the one query a global report runs, chosen by which filters the request carries
#[derive(Clone, Copy)]
pub enum ReportBranch<'request> {
    Scoped {
        column: ScopeColumn,
        value: &'request str,
    },
    TeamCustomer {
        team_id: &'request str,
        customer_id: &'request str,
    },
    Teams(Option<&'request str>),
    Customers,
    Keys,
}

#[async_trait]
pub trait GlobalReportStore: Send + Sync {
    async fn report(
        &self,
        branch: ReportBranch<'_>,
        window: ReportWindow<'_>,
    ) -> Result<GlobalReport, ScopedReportError>;
}

pub struct PostgresGlobalReportStore {
    pub reports: PostgresScopedReportStore,
}

#[async_trait]
impl GlobalReportStore for PostgresGlobalReportStore {
    async fn report(
        &self,
        branch: ReportBranch<'_>,
        window: ReportWindow<'_>,
    ) -> Result<GlobalReport, ScopedReportError> {
        let store = &self.reports.store;
        match branch {
            ReportBranch::Scoped { column, value } => {
                scoped_rows(&self.reports, column, value, window).await
            }
            ReportBranch::TeamCustomer {
                team_id,
                customer_id,
            } => get_spend_by_team_and_customer(store, window, team_id, customer_id)
                .await
                .map(GlobalReport::TeamsCustomers),
            ReportBranch::Teams(team_id) => {
                get_spend_by_team(store, window.start, window.end, team_id)
                    .await
                    .map(GlobalReport::Teams)
            }
            ReportBranch::Customers => {
                let query = Query {
                    sql: CUSTOMER_SQL,
                    parameters: vec![window.start, window.end],
                };
                decoded_rows(store, &query)
                    .await
                    .map(GlobalReport::Customers)
            }
            ReportBranch::Keys => {
                let query = Query {
                    sql: ALL_KEYS_SQL,
                    parameters: vec![window.start, window.end],
                };
                decoded_rows(store, &query).await.map(GlobalReport::Keys)
            }
        }
    }
}

// the api_key and internal_user_id branches run B1726's scoped report over the same window
async fn scoped_rows(
    reports: &PostgresScopedReportStore,
    column: ScopeColumn,
    value: &str,
    window: ReportWindow<'_>,
) -> Result<GlobalReport, ScopedReportError> {
    let request = ScopedReportRequest {
        column,
        start: window.start,
        end: window.end,
        value,
    };
    reports.scoped_report(request).await.map(GlobalReport::Keys)
}

// get_spend_by_team_and_customer: the team and the customer are both required and both bound as values
pub async fn get_spend_by_team_and_customer(
    store: &PostgresStore,
    window: ReportWindow<'_>,
    team_id: &str,
    customer_id: &str,
) -> Result<Vec<DailyTeamCustomerSpend>, ScopedReportError> {
    let query = Query {
        sql: TEAM_CUSTOMER_SQL,
        parameters: vec![window.start, window.end, &team_id, &customer_id],
    };
    decoded_rows(store, &query).await
}
