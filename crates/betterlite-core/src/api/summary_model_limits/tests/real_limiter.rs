use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::model_budget_limiter::ModelBudgetLimiter;
use betterlite_auth::model_budget_limiter::cache::ModelBudgetCache;
use betterlite_auth::model_budget_limiter::clock::BudgetClock;
use betterlite_auth::model_budget_values::candidates::BedrockCatalog;
use betterlite_auth::model_budget_values::spend_reads::CachedSpend;
use chrono::{DateTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;

use super::super::budget::summary_model_within_budget;
use super::support::principal;

#[derive(Debug, thiserror::Error)]
#[error("the test cache never fails")]
struct NeverFails;

struct SpendCache {
    spend: BTreeMap<String, Decimal>,
    reads: Mutex<Vec<String>>,
}

impl ModelBudgetCache for SpendCache {
    type Error = NeverFails;

    fn redis_configured(&self) -> bool {
        false
    }

    fn redis_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send {
        self.memory_spend(key)
    }

    fn memory_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send {
        self.reads.lock().unwrap().push(key.to_owned());
        ready(self.spend.get(key).copied().map(CachedSpend::Number))
    }

    fn increment_counter(
        &self,
        _key: &str,
        _ttl: Duration,
    ) -> impl Future<Output = Option<Decimal>> + Send {
        ready(None)
    }

    fn increment_spend(
        &self,
        _duration: Duration,
        _spend_key: &str,
        _start_time_key: &str,
        _cost: Decimal,
    ) -> impl Future<Output = Result<(), NeverFails>> + Send {
        ready(Ok(()))
    }

    fn flush_increments(&self) -> impl Future<Output = ()> + Send {
        ready(())
    }
}

struct OneDay;

impl DurationParser for OneDay {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        Some(86_400)
    }
}

struct PlainModels;

impl BedrockCatalog for PlainModels {
    fn base_model(&self, model: &str) -> String {
        model.to_owned()
    }

    fn prices_as_bedrock(&self, _base_model: &str) -> bool {
        false
    }
}

struct LongUptime;

impl BudgetClock for LongUptime {
    fn now(&self) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }

    fn uptime(&self) -> Duration {
        Duration::from_secs(1_000_000)
    }
}

#[rstest]
#[case::within(Decimal::ZERO, true)]
#[case::at_the_budget(Decimal::ONE, false)]
#[tokio::test]
async fn budget_scope_arguments_reach_the_shared_model_budget_limiter(
    #[case] spent: Decimal,
    #[case] allowed: bool,
) {
    let key = "virtual_key_spend:key-1:sum:1d".to_owned();
    let limiter = ModelBudgetLimiter {
        cache: SpendCache {
            spend: BTreeMap::from([(key.clone(), spent)]),
            reads: Mutex::new(Vec::new()),
        },
        durations: Box::new(OneDay),
        catalog: Box::new(PlainModels),
        clock: Box::new(LongUptime),
        budget_exceeded_status: 429,
    };
    let auth = principal(
        r#"{"token": "key-1", "model_max_budget": {"sum": {"budget_limit": 1.0, "time_period": "1d"}}}"#,
    );
    let checked = summary_model_within_budget(Some(&auth), Some(&limiter), "sum").await;
    assert_eq!(checked.unwrap(), allowed);
    assert_eq!(*limiter.cache.reads.lock().unwrap(), vec![key]);
}
