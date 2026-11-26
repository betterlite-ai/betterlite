#![cfg(test)]

use async_trait::async_trait;
use betterlite_auth::auth_model_budgets::global_spend::{
    GlobalSpendFetcher, ProxyBudgetAlert, ProxyBudgetAlertSink, ProxyBudgetRows,
    get_global_proxy_spend,
};
use betterlite_auth::auth_model_budgets::rows::SpendReadError;
use betterlite_cache::backend::{BackendError, CacheBackend, Ttl};
use betterlite_cache::coordinator::EventDrivenCacheCoordinator;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::task::TaskTracker;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::auth_model_budgets::Settings;

fn load(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .auth_model_budgets)
}

fn amount(text: &str) -> Decimal {
    text.parse().unwrap()
}

#[tokio::test]
async fn max_budget_default_zero() {
    let absent = load("litellm_settings: {}", &[]).expect("an empty section loads");
    assert_eq!(absent.max_budget, Decimal::ZERO);
    assert!(absent.max_budget <= Decimal::ZERO);
    let unset = load("model_list: []", &[]).expect("a config without the key loads");
    assert_eq!(unset, Settings::default());
    assert_eq!(observed_reads(absent).await, 0);
    assert_eq!(observed_reads(unset).await, 0);
}

#[tokio::test]
async fn positive_budget_enables_read() {
    let source = "\
litellm_settings:
  max_budget: os.environ/PROXY_MAX_BUDGET";
    let loaded = load(source, &[("PROXY_MAX_BUDGET", "250.50")])
        .expect("the environment resolves the ceiling");
    assert_eq!(loaded.max_budget, amount("250.50"));
    assert!(loaded.max_budget > Decimal::ZERO);
    let literal =
        load("litellm_settings:\n  max_budget: 100.0", &[]).expect("a literal ceiling loads");
    assert_eq!(literal.max_budget, amount("100.0"));
    assert!(literal.max_budget > Decimal::ZERO);
    assert_eq!(observed_reads(loaded).await, 1);
    assert_eq!(observed_reads(literal).await, 1);
}

// get_global_proxy_spend gates on `litellm.max_budget > 0`, so these values never read the aggregate
#[rstest]
#[case::zero("0")]
#[case::zero_float("0.0")]
#[case::negative("-5")]
#[case::nan(".nan")]
#[tokio::test]
async fn zero_negative_and_nan_skip_read(#[case] value: &str) {
    let source = format!("litellm_settings:\n  max_budget: {value}");
    let loaded = load(&source, &[]).expect("a non-positive ceiling still loads");
    let expected = if value == "-5" {
        Decimal::new(-5, 0)
    } else {
        Decimal::ZERO
    };
    assert_eq!(loaded.max_budget, expected, "{value}");
    assert!(loaded.max_budget <= Decimal::ZERO, "{value}");
    assert_eq!(observed_reads(loaded).await, 0);
}

#[rstest]
#[case::text("not-a-number")]
#[case::boolean("true")]
#[case::out_of_range("1.0e29")]
fn invalid_budget_rejected(#[case] value: &str) {
    let source = format!("litellm_settings:\n  max_budget: {value}");
    let error = load(&source, &[]).expect_err("a budget that is not money fails the load");
    assert!(
        error.to_string().contains("litellm_settings.max_budget"),
        "{error}"
    );
    assert_eq!(error.category(), "invalid_scalar");
}

#[derive(Default)]
struct Reads(AtomicUsize);

#[async_trait]
impl ProxyBudgetRows for Reads {
    async fn find_proxy_budget(&self) -> Result<Option<Decimal>, SpendReadError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Some(Decimal::ZERO))
    }
}

#[async_trait]
impl CacheBackend for Reads {
    fn default_ttl(&self) -> i64 {
        60
    }
    async fn async_get_cache(&self, _key: &str) -> Result<Option<CacheValue>, BackendError> {
        Ok(None)
    }
    async fn async_set_cache(
        &self,
        _key: &str,
        _value: CacheValue,
        _ttl: Ttl,
    ) -> Result<(), BackendError> {
        Ok(())
    }
}

impl ProxyBudgetAlertSink for Reads {
    fn schedule(&self, _alert: ProxyBudgetAlert) {}
}

async fn observed_reads(settings: Settings) -> usize {
    let reads = Arc::new(Reads::default());
    let fetcher = GlobalSpendFetcher {
        rows: Arc::<Reads>::clone(&reads),
        cache: Arc::<Reads>::clone(&reads),
        coordinator: EventDrivenCacheCoordinator::new(
            "[GLOBAL SPEND]".to_owned(),
            Duration::from_secs(1),
        ),
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
        default_deadline_seconds: 1.0,
        alerts: Arc::<Reads>::clone(&reads),
    };
    let spend = get_global_proxy_spend(settings.max_budget, Some(&fetcher), "admin", "token")
        .await
        .unwrap();
    assert_eq!(
        spend,
        (settings.max_budget > Decimal::ZERO).then_some(Decimal::ZERO)
    );
    fetcher.tasks.close();
    fetcher.tasks.wait().await;
    reads.0.load(Ordering::Relaxed)
}

#[rstest]
#[case::null("litellm_settings:\n  max_budget: null", vec![])]
#[case::empty_env("litellm_settings:\n  max_budget: os.environ/BUDGET", vec![("BUDGET", "")])]
#[tokio::test]
async fn max_budget_null_and_empty_env_default(
    #[case] source: &str,
    #[case] variables: Vec<(&str, &str)>,
) {
    let settings = load(source, &variables).unwrap();
    assert_eq!(settings.max_budget, Decimal::ZERO);
    assert_eq!(observed_reads(settings).await, 0);
}

#[test]
fn max_budget_missing_env_names_the_field() {
    let error = load("litellm_settings:\n  max_budget: os.environ/BUDGET", &[]).unwrap_err();
    assert!(
        matches!(error, ConfigError::MissingEnvironment { ref variable, ref path }
        if variable == "BUDGET" && path == "litellm_settings.max_budget")
    );
}
