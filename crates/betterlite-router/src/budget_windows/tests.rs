#![cfg(test)]

use std::time::Duration;

use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use super::budget::{
    BudgetDeployment, BudgetPeriod, DeploymentBudget, DeploymentLimits, ProviderBudget, TagBudget,
};
use super::keys::{WindowKeys, deployment_keys, period_label, provider_keys, tag_keys};
use super::service::{BudgetWindows, should_init_budget_limiter};
use super::test_support::{MemoryEntry, MemoryStore};
use super::window::{
    BudgetWindowStore as _, NewWindow, WindowWrite, get_or_set_budget_start_time,
    handle_new_budget_window, init_provider_budget_in_cache,
};

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

fn entry(store: &MemoryStore, key: &str) -> Option<MemoryEntry> {
    store.entries.lock().unwrap().get(key).cloned()
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
    let store = MemoryStore::default();
    let key = provider_keys("openai", &None).start_time;
    let start =
        get_or_set_budget_start_time(&store, &key, 100.0, Some(Duration::from_secs(60))).await;
    assert!(start.total_cmp(&100.0).is_eq());
    assert_eq!(
        entry(&store, &key),
        Some(MemoryEntry {
            start: Some(100.0),
            spend: None,
            ttl: Some(Duration::from_secs(60)),
        })
    );
    let cached = get_or_set_budget_start_time(&store, &key, 500.0, None).await;
    assert!(cached.total_cmp(&100.0).is_eq());
    assert_eq!(
        entry(&store, &key).and_then(|entry| entry.start),
        Some(100.0)
    );
    let observed = store.observe(&key, &key);
    assert!(observed.started);
    assert!(observed.spend.is_none());
    let pushed = MemoryStore::default();
    pushed.seed_start(&key, 0.0, Duration::from_secs(10));
    let zero = get_or_set_budget_start_time(&pushed, &key, 500.0, None).await;
    assert!(zero.total_cmp(&0.0).is_eq());
    assert_eq!(
        entry(&pushed, &key).and_then(|entry| entry.start),
        Some(0.0)
    );
}

async fn reset_window() {
    let store = MemoryStore::default();
    let names = deployment_keys("d1", &day());
    let start = handle_new_budget_window(
        &store,
        NewWindow {
            keys: &names,
            current_time: 100.0,
            response_cost: Decimal::from(2),
            ttl: Some(Duration::from_secs(30)),
        },
    )
    .await;
    assert!(start.total_cmp(&100.0).is_eq());
    assert_eq!(
        entry(&store, &names.spend).and_then(|entry| entry.spend),
        Some(Decimal::from(2))
    );
    assert_eq!(
        entry(&store, &names.start_time).and_then(|entry| entry.start),
        Some(100.0)
    );
    let observed = store.observe(&names.start_time, &names.spend);
    assert!(observed.started);
    assert_eq!(observed.spend, Some(Decimal::from(2)));
    let untimed = MemoryStore::default();
    let untimed_keys = WindowKeys {
        spend: "spend".to_owned(),
        start_time: "start".to_owned(),
    };
    handle_new_budget_window(
        &untimed,
        NewWindow {
            keys: &untimed_keys,
            current_time: 1.0,
            response_cost: Decimal::ONE,
            ttl: None,
        },
    )
    .await;
    assert_eq!(entry(&untimed, "spend").and_then(|entry| entry.ttl), None);
}

async fn provider_init() {
    let store = MemoryStore::default();
    let budget = ProviderBudget {
        budget_limit: Some(Decimal::from(3)),
        period: Some(day()),
    };
    init_provider_budget_in_cache(&store, "openai", &budget, 100.0).await;
    let names = provider_keys("openai", &Some(day()));
    assert_eq!(
        entry(&store, &names.spend).and_then(|entry| entry.spend),
        Some(Decimal::ZERO)
    );
    assert_eq!(
        entry(&store, &names.start_time).and_then(|entry| entry.start),
        Some(100.0)
    );
    store
        .write(
            &names.spend,
            WindowWrite::Spend(Decimal::from(7)),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();
    init_provider_budget_in_cache(&store, "openai", &budget, 999.0).await;
    assert_eq!(
        entry(&store, &names.start_time).and_then(|entry| entry.start),
        Some(100.0)
    );
    assert_eq!(
        entry(&store, &names.spend).and_then(|entry| entry.spend),
        Some(Decimal::from(7))
    );
    let untimed = MemoryStore::default();
    let timeless = ProviderBudget {
        budget_limit: None,
        period: None,
    };
    init_provider_budget_in_cache(&untimed, "azure", &timeless, 7.0).await;
    assert_eq!(
        entry(&untimed, &provider_keys("azure", &None).spend).and_then(|entry| entry.spend),
        Some(Decimal::ZERO)
    );
    assert_eq!(
        entry(&untimed, &provider_keys("azure", &None).spend).and_then(|entry| entry.ttl),
        None
    );
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
        Boundary::EmptyAndMissing => empty_and_missing(),
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
    let store = MemoryStore::default();
    let start = get_or_set_budget_start_time(&store, "start", 5.0, Some(Duration::ZERO)).await;
    assert!(start.total_cmp(&5.0).is_eq());
    assert_eq!(
        entry(&store, "start").and_then(|entry| entry.ttl),
        Some(Duration::ZERO)
    );
}

fn empty_and_missing() {
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
    assert!(
        windows
            .tag
            .as_ref()
            .and_then(|configs| configs.get("missing"))
            .is_none()
    );
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
}
