#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use super::budget::{
    BudgetDeployment, BudgetPeriod, DeploymentBudget, DeploymentLimits, ProviderBudget, TagBudget,
};
use super::keys::{deployment_keys, period_label, provider_keys, tag_keys};
use super::service::{BudgetWindows, should_init_budget_limiter};
use super::window::{get_or_set_budget_start_time, has_spend, init_provider_budget_in_cache};
use crate::budget_increment::dual::DualWindowCache;
use crate::budget_increment::error::IncrementError;
use crate::budget_increment::increment::BudgetIncrement;
use crate::budget_increment::queue::QueuedIncrement;
use crate::budget_increment::window::{BudgetConfig, WindowCache as _, WindowRedis};

#[derive(Default)]
struct FakeRedis {
    floats: Mutex<BTreeMap<String, f64>>,
    spend_writes: Mutex<Vec<(String, Decimal)>>,
    fail_reads: bool,
}

#[async_trait]
impl WindowRedis for FakeRedis {
    async fn read_start_time(&self, key: &str) -> Result<Option<f64>, IncrementError> {
        if self.fail_reads {
            return Err(IncrementError::InvalidCacheValue {
                key: key.to_owned(),
            });
        }
        Ok(self.floats.lock().unwrap().get(key).copied())
    }

    async fn write_start_time(&self, key: &str, start_time: f64, _ttl: Duration) {
        self.floats
            .lock()
            .unwrap()
            .insert(key.to_owned(), start_time);
    }

    async fn write_spend(&self, key: &str, cost: Decimal, _ttl: Duration) {
        self.spend_writes
            .lock()
            .unwrap()
            .push((key.to_owned(), cost));
    }

    async fn write_increments(&self, _operations: &[QueuedIncrement]) -> bool {
        true
    }
}

fn failing_redis() -> FakeRedis {
    FakeRedis {
        floats: Mutex::default(),
        spend_writes: Mutex::default(),
        fail_reads: true,
    }
}

fn redis_with(key: &str, value: f64) -> FakeRedis {
    let redis = FakeRedis::default();
    redis.floats.lock().unwrap().insert(key.to_owned(), value);
    redis
}

fn window_cache(clock: f64, redis: Option<FakeRedis>) -> DualWindowCache<FakeRedis> {
    let memory =
        LocalTtlCache::with_clock(None, None, None, 1024, Arc::new(move || clock)).unwrap();
    DualWindowCache {
        memory: Arc::new(Mutex::new(memory)),
        redis,
    }
}

fn stored(cache: &DualWindowCache<FakeRedis>, key: &str) -> Option<CacheValue> {
    cache.memory.lock().unwrap().get(key).unwrap()
}

fn spend_value(text: &str) -> CacheValue {
    CacheValue::Object(BTreeMap::from([(
        "spend".to_owned(),
        CacheValue::Text(text.to_owned()),
    )]))
}

fn day() -> BudgetPeriod {
    BudgetPeriod {
        spec: "1d".to_owned(),
        seconds: Duration::from_secs(86_400),
    }
}

fn deployment(model_id: &str, budget: i64) -> DeploymentLimits {
    DeploymentLimits {
        model_id: Some(model_id.to_owned()),
        max_budget: Some(Decimal::from(budget)),
        period: Some(day()),
    }
}

fn provider_config() -> IndexMap<String, ProviderBudget> {
    let mut configs = IndexMap::new();
    configs.insert(
        "openai".to_owned(),
        ProviderBudget {
            budget_limit: Some(Decimal::from(100)),
            period: Some(day()),
        },
    );
    configs
}

#[derive(Clone, Copy, Debug)]
enum Behavior {
    ConfiguredMaps,
    KeyTable,
    StartWindow,
    ResetWindow,
    ProviderInit,
    Registration,
}

#[rstest]
#[case::configured_maps(Behavior::ConfiguredMaps)]
#[case::key_table(Behavior::KeyTable)]
#[case::start_window(Behavior::StartWindow)]
#[case::reset_window(Behavior::ResetWindow)]
#[case::provider_init(Behavior::ProviderInit)]
#[case::registration(Behavior::Registration)]
#[tokio::test]
async fn budget_windows_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::ConfiguredMaps => configured_maps(),
        Behavior::KeyTable => key_table(),
        Behavior::StartWindow => start_window().await,
        Behavior::ResetWindow => reset_window().await,
        Behavior::ProviderInit => provider_init().await,
        Behavior::Registration => registration(),
    }
}

fn configured_maps() {
    let mut tags = IndexMap::new();
    tags.insert(
        "team".to_owned(),
        TagBudget {
            max_budget: Some(Decimal::from(5)),
            period: Some(day()),
        },
    );
    let windows = BudgetWindows::new(Some(provider_config()), Some(tags), &[deployment("d1", 10)]);
    assert!(should_init_budget_limiter(
        windows.provider.as_ref(),
        windows.tag.as_ref(),
        None
    ));
    assert_eq!(windows.deployment.as_ref().map(IndexMap::len), Some(1));
    assert_eq!(windows.tag.as_ref().map(IndexMap::len), Some(1));
    assert_eq!(
        windows
            .provider_budget("openai")
            .map(|budget| budget.budget_limit),
        Some(Some(Decimal::from(100)))
    );
    assert_eq!(
        windows
            .tag
            .as_ref()
            .and_then(|configs| configs.get("team"))
            .map(|budget| budget.max_budget),
        Some(Some(Decimal::from(5)))
    );
    assert_eq!(
        windows
            .deployment_budget("d1")
            .map(|budget| budget.max_budget),
        Some(Decimal::from(10))
    );
    let zero = DeploymentLimits {
        model_id: Some("zero".to_owned()),
        max_budget: Some(Decimal::ZERO),
        period: None,
    };
    assert!(should_init_budget_limiter(
        None,
        None,
        Some(std::slice::from_ref(&zero))
    ));
    assert!(should_init_budget_limiter(
        Some(&IndexMap::new()),
        None,
        None
    ));
    assert!(!should_init_budget_limiter(None, None, None));
    assert!(!should_init_budget_limiter(None, None, Some(&[])));
}

fn key_table() {
    let mut tags = IndexMap::new();
    tags.insert(
        "team".to_owned(),
        TagBudget {
            max_budget: None,
            period: None,
        },
    );
    let mut providers = IndexMap::new();
    providers.insert(
        "azure".to_owned(),
        ProviderBudget {
            budget_limit: Some(Decimal::from(1)),
            period: None,
        },
    );
    let windows = BudgetWindows::new(Some(providers), Some(tags), &[deployment("d1", 10)]);
    let deployments = [
        BudgetDeployment {
            model_id: Some("d1"),
            provider: Some("openai"),
        },
        BudgetDeployment {
            model_id: Some("d1"),
            provider: Some("azure"),
        },
        BudgetDeployment {
            model_id: None,
            provider: None,
        },
    ];
    let output = windows.cache_keys(&deployments, &["team".to_owned(), "other".to_owned()]);
    assert_eq!(
        output.keys,
        vec![
            "deployment_spend:d1:1d".to_owned(),
            "deployment_spend:d1:1d".to_owned(),
            "tag_spend:team:None".to_owned(),
        ]
    );
    assert_eq!(
        output.deployment_providers,
        vec![Some("openai".to_owned()), Some("azure".to_owned()), None]
    );
    assert_eq!(output.provider_configs.len(), 1);
    assert_eq!(output.deployment_configs.len(), 1);
    assert_eq!(period_label(&Some(day())), "1d");
    assert_eq!(period_label(&None), "None");
    assert_eq!(
        provider_keys("openai", &Some(day())).spend,
        "provider_spend:openai:1d"
    );
    assert_eq!(
        provider_keys("openai", &None).start_time,
        "provider_budget_start_time:openai"
    );
    assert_eq!(
        deployment_keys("d1", &day()).start_time,
        "deployment_budget_start_time:d1"
    );
    assert_eq!(
        tag_keys("team", &None).start_time,
        "tag_budget_start_time:team"
    );
}

async fn start_window() {
    let cache = window_cache(0.0, None);
    let key = provider_keys("openai", &None).start_time;
    let start = get_or_set_budget_start_time(&cache, &key, 100.0, Duration::from_secs(60)).await;
    assert!(start.total_cmp(&100.0).is_eq());
    assert_eq!(stored(&cache, &key), Some(CacheValue::Float(100.0)));
    assert_eq!(
        cache.memory.lock().unwrap().ttl_entries(),
        vec![(key.clone(), 60.0_f64)]
    );
    let cached = get_or_set_budget_start_time(&cache, &key, 500.0, Duration::from_secs(60)).await;
    assert!(cached.total_cmp(&100.0).is_eq());
    assert_eq!(stored(&cache, &key), Some(CacheValue::Float(100.0)));
    let pushed = window_cache(0.0, Some(redis_with(&key, 0.0)));
    let zero = get_or_set_budget_start_time(&pushed, &key, 500.0, Duration::from_secs(60)).await;
    assert!(zero.total_cmp(&0.0).is_eq());
    cache
        .memory
        .lock()
        .unwrap()
        .set(&key, CacheValue::Text("broken".to_owned()), None)
        .unwrap();
    let restarted = get_or_set_budget_start_time(&cache, &key, 7.0, Duration::from_secs(60)).await;
    assert!(restarted.total_cmp(&7.0).is_eq());
    assert_eq!(stored(&cache, &key), Some(CacheValue::Float(7.0)));
}

// _handle_new_budget_window lives in the shared increment service, which an expired start sends there
async fn reset_window() {
    let cache = Arc::new(window_cache(0.0, None));
    let names = deployment_keys("d1", &day());
    cache
        .write_start_time(&names.start_time, 0.0, day().seconds)
        .await
        .unwrap();
    cache
        .reset_spend(&names.spend, Decimal::from(9), day().seconds)
        .await
        .unwrap();
    let increment = BudgetIncrement {
        cache: Arc::clone(&cache),
        clock: Arc::new(|| 100_000.0_f64),
        queue: Arc::default(),
        flush_lock: Arc::default(),
    };
    let config = BudgetConfig {
        max_budget: Some(Decimal::from(10)),
        budget_duration: Some(day().seconds),
    };
    increment
        .increment_spend_for_key(&config, &names.spend, &names.start_time, Decimal::from(2))
        .await
        .unwrap();
    assert_eq!(stored(&cache, &names.spend), Some(spend_value("2")));
    assert_eq!(
        stored(&cache, &names.start_time),
        Some(CacheValue::Float(100_000.0))
    );
}

async fn provider_init() {
    let cache = window_cache(0.0, None);
    let budget = ProviderBudget {
        budget_limit: Some(Decimal::from(3)),
        period: Some(day()),
    };
    init_provider_budget_in_cache(&cache, "openai", &budget, 100.0).await;
    let names = provider_keys("openai", &Some(day()));
    assert_eq!(stored(&cache, &names.spend), Some(spend_value("0")));
    assert_eq!(
        stored(&cache, &names.start_time),
        Some(CacheValue::Float(100.0))
    );
    cache
        .reset_spend(&names.spend, Decimal::from(7), day().seconds)
        .await
        .unwrap();
    init_provider_budget_in_cache(&cache, "openai", &budget, 999.0).await;
    assert_eq!(
        stored(&cache, &names.start_time),
        Some(CacheValue::Float(100.0))
    );
    assert_eq!(stored(&cache, &names.spend), Some(spend_value("7")));
    let cold = window_cache(0.0, Some(redis_with(&names.spend, 7.0)));
    init_provider_budget_in_cache(&cold, "openai", &budget, 100.0).await;
    assert_eq!(stored(&cold, &names.spend), None);
    let cold_writes = cold
        .redis
        .as_ref()
        .unwrap()
        .spend_writes
        .lock()
        .unwrap()
        .clone();
    assert!(cold_writes.is_empty());
    let failing = window_cache(0.0, Some(failing_redis()));
    init_provider_budget_in_cache(&failing, "openai", &budget, 100.0).await;
    assert_eq!(stored(&failing, &names.spend), Some(spend_value("0")));
    let failing_writes = failing
        .redis
        .as_ref()
        .unwrap()
        .spend_writes
        .lock()
        .unwrap()
        .clone();
    assert_eq!(failing_writes, vec![(names.spend.clone(), Decimal::ZERO)]);
    let untimed = window_cache(0.0, None);
    let timeless = ProviderBudget {
        budget_limit: Some(Decimal::from(3)),
        period: None,
    };
    init_provider_budget_in_cache(&untimed, "azure", &timeless, 7.0).await;
    let azure = provider_keys("azure", &None);
    assert_eq!(stored(&untimed, &azure.spend), None);
    assert_eq!(stored(&untimed, &azure.start_time), None);
}

fn registration() {
    let mut tags = IndexMap::new();
    tags.insert(
        "team".to_owned(),
        TagBudget {
            max_budget: Some(Decimal::from(5)),
            period: Some(day()),
        },
    );
    let mut windows = BudgetWindows::new(Some(provider_config()), Some(tags), &[]);
    assert!(windows.deployment.as_ref().is_none());
    let incomplete = DeploymentLimits {
        model_id: Some("d1".to_owned()),
        max_budget: Some(Decimal::from(1)),
        period: None,
    };
    windows.register_deployment_budget(&incomplete);
    assert!(windows.deployment.as_ref().is_none());
    windows.register_deployment_budget(&deployment("d1", 4));
    assert_eq!(windows.deployment.as_ref().map(IndexMap::len), Some(1));
    assert_eq!(windows.tag.as_ref().map(IndexMap::len), Some(1));
    assert_eq!(
        windows
            .provider_budget("openai")
            .map(|budget| budget.budget_limit),
        Some(Some(Decimal::from(100)))
    );
    assert_eq!(
        windows
            .tag
            .as_ref()
            .and_then(|configs| configs.get("team"))
            .map(|budget| budget.max_budget),
        Some(Some(Decimal::from(5)))
    );
    windows.sync_deployment_budget(&deployment("d1", 9));
    assert_eq!(
        windows
            .deployment_budget("d1")
            .map(|budget| budget.max_budget),
        Some(Decimal::from(9))
    );
    windows.sync_deployment_budget(&incomplete);
    assert!(windows.deployment.as_ref().is_none());
    windows.unregister_deployment_budget("gone");
    windows.register_deployment_budget(&deployment("d2", 1));
    windows.unregister_deployment_budget("d2");
    assert!(windows.deployment.as_ref().is_none());
    let mut untouched = BudgetWindows::default();
    untouched.unregister_deployment_budget("d1");
    assert!(untouched.deployment.as_ref().is_none());
    let unknown = DeploymentLimits {
        model_id: None,
        max_budget: Some(Decimal::from(1)),
        period: Some(day()),
    };
    untouched.sync_deployment_budget(&unknown);
    assert!(untouched.deployment.as_ref().is_none());
}

#[derive(Clone, Copy, Debug)]
enum Boundary {
    ExactKeys,
    ZeroBudgets,
    EmptyAndMissing,
}

#[rstest]
#[case::exact_keys(Boundary::ExactKeys)]
#[case::zero_budgets(Boundary::ZeroBudgets)]
#[case::empty_and_missing(Boundary::EmptyAndMissing)]
#[tokio::test]
async fn budget_windows_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::ExactKeys => exact_keys(),
        Boundary::ZeroBudgets => zero_budgets().await,
        Boundary::EmptyAndMissing => empty_and_missing().await,
    }
}

fn exact_keys() {
    let mut providers = IndexMap::new();
    providers.insert(
        "openai".to_owned(),
        ProviderBudget {
            budget_limit: None,
            period: Some(BudgetPeriod {
                spec: "24h".to_owned(),
                seconds: Duration::from_secs(86_400),
            }),
        },
    );
    let windows = BudgetWindows::new(Some(providers), None, &[]);
    let output = windows.cache_keys(
        &[BudgetDeployment {
            model_id: Some("d1"),
            provider: Some("openai"),
        }],
        &[],
    );
    assert_eq!(output.keys, vec!["provider_spend:openai:24h".to_owned()]);
    assert_eq!(output.deployment_providers, vec![Some("openai".to_owned())]);
    assert!(output.deployment_configs.is_empty());
    assert_eq!(
        provider_keys("openai", &Some(day())).spend,
        "provider_spend:openai:1d"
    );
}

async fn zero_budgets() {
    let windows = BudgetWindows::new(
        None,
        None,
        &[DeploymentLimits {
            model_id: Some("d1".to_owned()),
            max_budget: Some(Decimal::ZERO),
            period: Some(BudgetPeriod {
                spec: "1s".to_owned(),
                seconds: Duration::from_secs(1),
            }),
        }],
    );
    assert_eq!(
        windows
            .deployment_budget("d1")
            .map(|budget| budget.max_budget),
        Some(Decimal::ZERO)
    );
    let cache = window_cache(0.0, None);
    let start = get_or_set_budget_start_time(&cache, "start", 5.0, Duration::ZERO).await;
    assert!(start.total_cmp(&5.0).is_eq());
    assert_eq!(
        cache.memory.lock().unwrap().ttl_entries(),
        vec![("start".to_owned(), 0.0_f64)]
    );
}

async fn empty_and_missing() {
    let windows = BudgetWindows::new(None, Some(IndexMap::new()), &[]);
    assert!(windows.provider.as_ref().is_none());
    assert!(
        windows
            .tag
            .as_ref()
            .is_some_and(|configs| configs.is_empty())
    );
    assert!(windows.deployment.as_ref().is_none());
    assert!(windows.deployment_budget("missing").is_none());
    assert!(windows.provider_budget("missing").is_none());
    let output = windows.cache_keys(&[], &["missing".to_owned()]);
    assert!(output.keys.is_empty());
    assert!(output.deployment_providers.is_empty());
    let empty = BudgetWindows::new(None, None, &[]);
    assert!(!should_init_budget_limiter(
        empty.provider.as_ref(),
        empty.tag.as_ref(),
        Some(&[])
    ));
    let budget = DeploymentBudget {
        max_budget: Decimal::ONE,
        period: day(),
    };
    assert_eq!(budget.period.spec, "1d");
    assert!(
        !has_spend(&window_cache(0.0, Some(FakeRedis::default())), "spend")
            .await
            .unwrap()
    );
    assert!(
        has_spend(&window_cache(0.0, Some(redis_with("spend", 0.0))), "spend")
            .await
            .unwrap()
    );
    assert!(
        has_spend(&window_cache(0.0, Some(failing_redis())), "spend")
            .await
            .is_err()
    );
    let broken = window_cache(f64::NAN, None);
    assert!(has_spend(&broken, "spend").await.is_err());
    let start = get_or_set_budget_start_time(&broken, "start", 3.0, Duration::from_secs(1)).await;
    assert!(start.total_cmp(&3.0).is_eq());
    init_provider_budget_in_cache(&broken, "openai", &provider_config()["openai"], 3.0).await;
    assert!(broken.memory.lock().unwrap().ttl_entries().is_empty());
}
