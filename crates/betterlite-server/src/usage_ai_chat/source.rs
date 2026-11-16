use std::future::Future;

use betterlite_analytics::daily_metrics::DailySpendPage;
use betterlite_db::daily_activity_contracts::DailyActivityProxyReads;
use betterlite_db::daily_activity_contracts::DailyActivityScope;
use betterlite_db::daily_activity_contracts::identifiers::DailyActivityTable;
use betterlite_db::daily_activity_repository::DailyActivityRepository;
use betterlite_db::daily_activity_repository::database::DailyActivityDatabase;
use chrono::{DateTime, Utc};

use super::constants::{ENTITY_FIELD_USER, PAGINATED_PAGE_SIZE, USAGE_TOP_API_KEYS_DEFAULT};
use super::error::UsageChatError;
use crate::daily_activity::pages::read::{
    DailyActivityPageRequest, PageCallbacks, get_daily_activity,
};
use crate::daily_activity::recovery::scope::{ScopeIds, ScopeRequest, daily_activity_scope};
use crate::daily_activity::rollups::get_daily_activity_aggregated;

pub type PageResult = Result<DailySpendPage, UsageChatError>;
pub type Ids<'text> = Option<&'text [String]>;

pub trait UsageDataSource: Sync {
    fn global(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: Option<&str>,
    ) -> impl Future<Output = PageResult> + Send;

    fn teams(
        &self,
        start_date: &str,
        end_date: &str,
        team_ids: Ids<'_>,
    ) -> impl Future<Output = PageResult> + Send;

    fn tags(
        &self,
        start_date: &str,
        end_date: &str,
        tags: Ids<'_>,
    ) -> impl Future<Output = PageResult> + Send;
}

pub trait UsageActivityRepository: Sync {
    fn aggregated(&self, scope: &DailyActivityScope) -> impl Future<Output = PageResult> + Send;

    fn page(
        &self,
        request: &DailyActivityPageRequest<'_>,
    ) -> impl Future<Output = PageResult> + Send;
}

pub struct DailyActivityUsageData<Repository> {
    pub repository: Option<Repository>,
}

impl<Repository> DailyActivityUsageData<Repository>
where
    Repository: UsageActivityRepository,
{
    fn connected(&self) -> Result<&Repository, UsageChatError> {
        self.repository
            .as_ref()
            .ok_or(UsageChatError::DatabaseNotConnected)
    }

    async fn global_scope(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: Option<&str>,
    ) -> PageResult {
        let scope = daily_activity_scope(ScopeRequest {
            table: DailyActivityTable::User,
            entity_id_field: ENTITY_FIELD_USER,
            entity_id: user_id.map(|id| ScopeIds::One(id.to_owned())),
            exclude_entity_ids: Vec::new(),
            api_key: None,
            start_date,
            end_date,
            model: None,
            timezone_offset_minutes: None,
            include_current_utc_day: false,
        })?;
        self.connected()?.aggregated(&scope).await
    }

    async fn entity_page(
        &self,
        table: DailyActivityTable,
        ids: Ids<'_>,
        start_date: &str,
        end_date: &str,
    ) -> PageResult {
        let entity_id_field = table.metadata().entity_field;
        let request = page_request(table, entity_id_field, ids, start_date, end_date);
        self.connected()?.page(&request).await
    }
}

impl<Repository> UsageDataSource for DailyActivityUsageData<Repository>
where
    Repository: UsageActivityRepository,
{
    async fn global(&self, start_date: &str, end_date: &str, user_id: Option<&str>) -> PageResult {
        self.global_scope(start_date, end_date, user_id).await
    }

    async fn teams(&self, start_date: &str, end_date: &str, team_ids: Ids<'_>) -> PageResult {
        self.entity_page(DailyActivityTable::Team, team_ids, start_date, end_date)
            .await
    }

    async fn tags(&self, start_date: &str, end_date: &str, tags: Ids<'_>) -> PageResult {
        self.entity_page(DailyActivityTable::Tag, tags, start_date, end_date)
            .await
    }
}

impl<Database, Reads, Clock> UsageActivityRepository
    for DailyActivityRepository<Database, Reads, Clock>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads + Send + Sync,
    Clock: Fn() -> DateTime<Utc> + Send + Sync,
{
    async fn aggregated(&self, scope: &DailyActivityScope) -> PageResult {
        get_daily_activity_aggregated(self, scope, None, false, USAGE_TOP_API_KEYS_DEFAULT)
            .await
            .map_err(|error| UsageChatError::Analytics {
                status_code: error.status_code,
                message: error.detail.error,
            })
    }

    async fn page(&self, request: &DailyActivityPageRequest<'_>) -> PageResult {
        get_daily_activity(self, request)
            .await
            .map_err(UsageChatError::from)
    }
}

fn page_request<'text>(
    table: DailyActivityTable,
    entity_id_field: &'text str,
    entity_ids: Ids<'text>,
    start_date: &'text str,
    end_date: &'text str,
) -> DailyActivityPageRequest<'text> {
    DailyActivityPageRequest {
        table,
        entity_id_field,
        entity_id: entity_ids.map(|ids| ScopeIds::Many(ids.to_vec())),
        exclude_entity_ids: Vec::new(),
        start_date: Some(start_date),
        end_date: Some(end_date),
        model: None,
        api_key: None,
        page: 1,
        page_size: PAGINATED_PAGE_SIZE,
        timezone_offset_minutes: None,
        include_current_utc_day: false,
        entity_metadata: None,
        callbacks: PageCallbacks::default(),
    }
}
