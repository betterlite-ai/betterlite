use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_model_budgets::global_spend::{
    COORDINATOR_WAIT, GLOBAL_SPEND_LOG_PREFIX, GlobalSpendFetcher, ProxyBudgetAlert,
    get_global_proxy_spend,
};
use betterlite_cache::coordinator::EventDrivenCacheCoordinator;
use betterlite_config::sections::auth_model_budgets::Settings;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio_util::task::TaskTracker;

use super::Call;
use super::doubles::{AlertRecorder, SpendCache, SpendRows};
use super::support::{Failure, decimal_json};

// get_global_proxy_spend: one read per cache miss, the null row is cached too, and only a known spend alerts
pub(super) async fn global_spend_value(call: &Call) -> Result<Value, Failure> {
    let spend_case = SpendCase::build(call)?;
    let (first, second) = spend_case.run(call).await?;
    Ok(json!({
        "spend": first,
        "second": second,
        "alerts": spend_case.alerts_json(),
        "reads": spend_case.rows.calls.load(Ordering::SeqCst),
    }))
}

struct SpendCase {
    settings: Settings,
    fetcher: GlobalSpendFetcher,
    rows: Arc<SpendRows>,
    alerts: Arc<AlertRecorder>,
}

impl SpendCase {
    fn build(call: &Call) -> Result<Self, Failure> {
        let tokens = call
            .tokens
            .clone()
            .ok_or(Failure::Missing("the call tokens"))?;
        let max_budget = call.max_budget.ok_or(Failure::Missing("the max budget"))?;
        let rows = Arc::new(SpendRows {
            spend: call.spend,
            gated: tokens.len() > 1,
            started: Semaphore::new(0),
            release: Semaphore::new(0),
            calls: AtomicUsize::new(0),
        });
        let alerts = Arc::new(AlertRecorder::default());
        let fetcher = GlobalSpendFetcher {
            rows: Arc::clone(&rows),
            cache: Arc::new(SpendCache::default()),
            coordinator: EventDrivenCacheCoordinator::new(
                GLOBAL_SPEND_LOG_PREFIX.to_owned(),
                COORDINATOR_WAIT,
            ),
            tracker: Arc::new(DbLookupStallTracker {
                clock: SystemClock,
                last_hit: Mutex::new(None),
            }),
            tasks: TaskTracker::new(),
            default_deadline_seconds: 5.0,
            alerts: Arc::clone(&alerts),
        };
        Ok(Self {
            settings: Settings { max_budget },
            fetcher,
            rows,
            alerts,
        })
    }

    async fn run(&self, call: &Call) -> Result<(Value, Value), Failure> {
        let tokens = call
            .tokens
            .as_ref()
            .ok_or(Failure::Missing("the call tokens"))?;
        let first_token = tokens.first().ok_or(Failure::Missing("the call tokens"))?;
        let admin = call
            .admin_name
            .as_deref()
            .ok_or(Failure::Missing("the admin name"))?;
        let fetcher = call.database.unwrap_or(true).then_some(&self.fetcher);
        if !self.rows.gated {
            let first = token_spend(&self.settings, fetcher, admin, first_token).await?;
            return Ok((first, Value::Null));
        }
        let follower_token = tokens.get(1).unwrap_or(first_token);
        let leader = get_global_proxy_spend(&self.settings, fetcher, admin, first_token);
        let follower = async {
            self.rows.wait_until_started().await;
            self.rows.release.add_permits(1);
            get_global_proxy_spend(&self.settings, fetcher, admin, follower_token).await
        };
        let (leader_result, follower_result) = tokio::join!(leader, follower);
        let first = leader_result
            .map_err(|_error| Failure::Coordinator)?
            .map_or(Value::Null, decimal_json);
        let second = follower_result
            .map_err(|_error| Failure::Coordinator)?
            .map_or(Value::Null, decimal_json);
        Ok((first, second))
    }

    fn alerts_json(&self) -> Vec<Value> {
        let alerts = mem::take(&mut *self.alerts.lock().unwrap_or_else(PoisonError::into_inner));
        alerts
            .into_iter()
            .map(|alert| {
                json!({
                    "type": ProxyBudgetAlert::TYPE,
                    "info": {
                        "user_id": alert.admin_name,
                        "max_budget": decimal_json(alert.max_budget),
                        "spend": decimal_json(alert.spend),
                        "token": alert.token,
                        "event_group": "proxy",
                    }
                })
            })
            .collect()
    }
}

async fn token_spend(
    settings: &Settings,
    fetcher: Option<&GlobalSpendFetcher>,
    admin: &str,
    token: &str,
) -> Result<Value, Failure> {
    let spend = get_global_proxy_spend(settings, fetcher, admin, token)
        .await
        .map_err(|_error| Failure::Coordinator)?;
    Ok(spend.map_or(Value::Null, decimal_json))
}
