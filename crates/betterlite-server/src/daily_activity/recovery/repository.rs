use betterlite_db::daily_activity_repository::DailyActivityRepository;
use betterlite_db::key_attribution::history::recovery::SpendLogHistory;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, Utc};
use std::sync::Arc;

use super::reads::ProxyDailyActivityReads;

pub fn daily_activity_repository<Database>(
    database: Database,
    store: PostgresStore,
    history: Arc<SpendLogHistory>,
) -> DailyActivityRepository<Database, ProxyDailyActivityReads, fn() -> DateTime<Utc>> {
    DailyActivityRepository {
        database,
        proxy_reads: ProxyDailyActivityReads { store, history },
        clock: Utc::now,
    }
}
