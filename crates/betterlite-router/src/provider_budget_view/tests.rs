#![cfg(test)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::provider_budget_view::ProviderBudgetConfig;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use super::cache::SpendCache;
use super::error::ProviderBudgetViewError;
use super::limiters::{CallbackKind, OptionalCallback, select_router_budget_limiter};
use super::view::{ProviderBudgetReads, ProviderBudgetView};

struct FakeCache {
    spend: Option<Decimal>,
    spend_fails: bool,
    expiry: Option<f64>,
    keys: Mutex<Vec<String>>,
}

#[async_trait]
impl SpendCache for FakeCache {
    async fn spend(&self, key: &str) -> Result<Option<Decimal>, ProviderBudgetViewError> {
        self.keys.lock().unwrap().push(key.to_owned());
        if self.spend_fails {
            return Err(ProviderBudgetViewError::Spend {
                key: key.to_owned(),
            });
        }
        Ok(self.spend)
    }

    async fn expiry_seconds(&self, key: &str) -> Option<f64> {
        self.keys.lock().unwrap().push(key.to_owned());
        self.expiry
    }
}

struct FakeReads {
    spend: Option<Decimal>,
    reset_at: Option<String>,
}

#[async_trait]
impl ProviderBudgetReads for FakeReads {
    async fn current_provider_spend(
        &self,
        _provider: &str,
    ) -> Result<Option<Decimal>, ProviderBudgetViewError> {
        Ok(self.spend)
    }

    async fn provider_budget_reset_at(
        &self,
        _provider: &str,
        _now: DateTime<Utc>,
    ) -> Result<Option<String>, ProviderBudgetViewError> {
        Ok(self.reset_at.clone())
    }
}

fn reads(spend: Option<Decimal>, reset_at: Option<String>) -> Arc<dyn ProviderBudgetReads> {
    Arc::new(FakeReads { spend, reset_at })
}

fn config(entries: &[(&str, Option<&str>)]) -> IndexMap<String, ProviderBudgetConfig> {
    entries
        .iter()
        .map(|(provider, duration)| {
            (
                (*provider).to_owned(),
                ProviderBudgetConfig {
                    max_budget: None,
                    budget_duration: duration.map(str::to_owned),
                },
            )
        })
        .collect()
}

fn no_spend_cache() -> FakeCache {
    FakeCache {
        spend: None,
        spend_fails: false,
        expiry: None,
        keys: Mutex::new(Vec::new()),
    }
}

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn selects_the_cached_router_budget_limiter_first() {
    let cached = reads(Some(Decimal::new(1, 0)), None);
    let later = OptionalCallback {
        kind: CallbackKind::RouterBudget,
        reads: reads(Some(Decimal::new(2, 0)), None),
    };
    let selected = select_router_budget_limiter(Some(Arc::clone(&cached)), &[later]).unwrap();
    assert!(Arc::ptr_eq(&selected, &cached));
}

#[test]
fn selects_the_first_router_budget_callback_when_none_is_cached() {
    let key_budget = OptionalCallback {
        kind: CallbackKind::KeyBudget,
        reads: reads(Some(Decimal::new(1, 0)), None),
    };
    let first = reads(Some(Decimal::new(2, 0)), None);
    let callbacks = [
        key_budget,
        OptionalCallback {
            kind: CallbackKind::RouterBudget,
            reads: Arc::clone(&first),
        },
        OptionalCallback {
            kind: CallbackKind::RouterBudget,
            reads: reads(Some(Decimal::new(3, 0)), None),
        },
    ];
    let selected = select_router_budget_limiter(None, &callbacks).unwrap();
    assert!(Arc::ptr_eq(&selected, &first));
}

#[test]
fn never_selects_a_key_budget_callback() {
    let callbacks = [
        OptionalCallback {
            kind: CallbackKind::KeyBudget,
            reads: reads(Some(Decimal::new(1, 0)), None),
        },
        OptionalCallback {
            kind: CallbackKind::Other,
            reads: reads(Some(Decimal::new(2, 0)), None),
        },
    ];
    assert!(select_router_budget_limiter(None, &callbacks).is_none());
}

#[test]
fn selects_no_limiter_without_any_callback() {
    assert!(select_router_budget_limiter(None, &[]).is_none());
}

#[tokio::test]
async fn reads_the_provider_spend_from_redis_when_attached() {
    let redis = Arc::new(FakeCache {
        spend: Some(Decimal::new(5, 0)),
        spend_fails: false,
        expiry: None,
        keys: Mutex::new(Vec::new()),
    });
    let dual = Arc::new(FakeCache {
        spend: Some(Decimal::new(9, 0)),
        spend_fails: false,
        expiry: None,
        keys: Mutex::new(Vec::new()),
    });
    let view = ProviderBudgetView {
        provider_budget_config: Some(config(&[("openai", Some("1d"))])),
        redis: Some(Arc::clone(&redis)),
        dual: Arc::clone(&dual),
    };
    assert_eq!(
        view.current_provider_spend("openai").await.unwrap(),
        Some(Decimal::new(5, 0))
    );
    assert!(dual.keys.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reads_the_provider_spend_from_the_dual_cache_without_redis() {
    let dual = Arc::new(FakeCache {
        spend: Some(Decimal::new(9, 0)),
        spend_fails: false,
        expiry: None,
        keys: Mutex::new(Vec::new()),
    });
    let view = ProviderBudgetView {
        provider_budget_config: Some(config(&[("openai", Some("1d"))])),
        redis: None,
        dual: Arc::clone(&dual),
    };
    assert_eq!(
        view.current_provider_spend("openai").await.unwrap(),
        Some(Decimal::new(9, 0))
    );
    assert_eq!(
        *dual.keys.lock().unwrap(),
        ["provider_spend:openai:1d".to_owned()]
    );
}

#[tokio::test]
async fn a_missing_provider_spend_reads_as_zero() {
    let dual = Arc::new(no_spend_cache());
    let view = ProviderBudgetView {
        provider_budget_config: Some(config(&[("openai", Some("1d"))])),
        redis: None,
        dual: Arc::clone(&dual),
    };
    assert_eq!(
        view.current_provider_spend("openai").await.unwrap(),
        Some(Decimal::ZERO)
    );
}

#[tokio::test]
async fn an_unconfigured_provider_reads_no_spend() {
    let dual = Arc::new(no_spend_cache());
    let view = ProviderBudgetView {
        provider_budget_config: Some(config(&[("openai", Some("1d"))])),
        redis: None,
        dual: Arc::clone(&dual),
    };
    assert_eq!(
        view.current_provider_spend("anthropic").await.unwrap(),
        None
    );
    assert!(dual.keys.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_null_provider_config_reads_no_spend() {
    let dual = Arc::new(no_spend_cache());
    let view = ProviderBudgetView {
        provider_budget_config: None,
        redis: None,
        dual: Arc::clone(&dual),
    };
    assert_eq!(view.current_provider_spend("openai").await.unwrap(), None);
    assert_eq!(
        view.provider_budget_reset_at("openai", Utc::now())
            .await
            .unwrap(),
        None
    );
    assert!(dual.keys.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_redis_spend_error_does_not_fall_back_to_local_spend() {
    let redis = Arc::new(FakeCache {
        spend: Some(Decimal::new(5, 0)),
        spend_fails: true,
        expiry: None,
        keys: Mutex::new(Vec::new()),
    });
    let dual = Arc::new(FakeCache {
        spend: Some(Decimal::new(9, 0)),
        spend_fails: false,
        expiry: None,
        keys: Mutex::new(Vec::new()),
    });
    let view = ProviderBudgetView {
        provider_budget_config: Some(config(&[("openai", Some("1d"))])),
        redis: Some(Arc::clone(&redis)),
        dual: Arc::clone(&dual),
    };
    let error = view.current_provider_spend("openai").await.unwrap_err();
    assert!(matches!(
        error,
        ProviderBudgetViewError::Spend { key } if key == "provider_spend:openai:1d"
    ));
    assert!(dual.keys.lock().unwrap().is_empty());
}

#[tokio::test]
async fn keys_the_spend_with_none_without_a_duration() {
    let dual = Arc::new(no_spend_cache());
    let view = ProviderBudgetView {
        provider_budget_config: Some(config(&[("openai", None)])),
        redis: None,
        dual: Arc::clone(&dual),
    };
    view.current_provider_spend("openai").await.unwrap();
    assert_eq!(
        *dual.keys.lock().unwrap(),
        ["provider_spend:openai:None".to_owned()]
    );
}

#[tokio::test]
async fn a_null_expiry_reads_no_reset_time() {
    let dual = Arc::new(no_spend_cache());
    let view = ProviderBudgetView {
        provider_budget_config: Some(config(&[("openai", Some("1d"))])),
        redis: None,
        dual: Arc::clone(&dual),
    };
    assert_eq!(
        view.provider_budget_reset_at("openai", instant("2026-11-19T12:00:00Z"))
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn adds_a_positive_expiry_to_the_injected_now() {
    let dual = Arc::new(FakeCache {
        spend: None,
        spend_fails: false,
        expiry: Some(90.0),
        keys: Mutex::new(Vec::new()),
    });
    let view = ProviderBudgetView {
        provider_budget_config: Some(config(&[("openai", Some("1d"))])),
        redis: None,
        dual: Arc::clone(&dual),
    };
    assert_eq!(
        view.provider_budget_reset_at("openai", instant("2026-11-19T12:00:00Z"))
            .await
            .unwrap(),
        Some("2026-11-19T12:01:30+00:00".to_owned())
    );
}

#[tokio::test]
async fn adds_a_negative_expiry_to_the_injected_now() {
    let dual = Arc::new(FakeCache {
        spend: None,
        spend_fails: false,
        expiry: Some(-30.0),
        keys: Mutex::new(Vec::new()),
    });
    let view = ProviderBudgetView {
        provider_budget_config: Some(config(&[("openai", Some("1d"))])),
        redis: None,
        dual: Arc::clone(&dual),
    };
    assert_eq!(
        view.provider_budget_reset_at("openai", instant("2026-11-19T12:00:00Z"))
            .await
            .unwrap(),
        Some("2026-11-19T11:59:30+00:00".to_owned())
    );
}

#[tokio::test]
async fn prints_microseconds_only_when_the_reset_time_has_them() {
    let dual = Arc::new(FakeCache {
        spend: None,
        spend_fails: false,
        expiry: Some(0.0),
        keys: Mutex::new(Vec::new()),
    });
    let view = ProviderBudgetView {
        provider_budget_config: Some(config(&[("openai", Some("1d"))])),
        redis: None,
        dual: Arc::clone(&dual),
    };
    assert_eq!(
        view.provider_budget_reset_at("openai", instant("2026-11-19T12:00:00.500Z"))
            .await
            .unwrap(),
        Some("2026-11-19T12:00:00.500000+00:00".to_owned())
    );
}

#[tokio::test]
async fn rounds_a_fractional_expiry_half_to_even_microseconds() {
    let dual = Arc::new(FakeCache {
        spend: None,
        spend_fails: false,
        expiry: Some(0.0000015),
        keys: Mutex::new(Vec::new()),
    });
    let view = ProviderBudgetView {
        provider_budget_config: Some(config(&[("openai", Some("1d"))])),
        redis: None,
        dual: Arc::clone(&dual),
    };
    assert_eq!(
        view.provider_budget_reset_at("openai", instant("2026-11-19T12:00:00Z"))
            .await
            .unwrap(),
        Some("2026-11-19T12:00:00.000002+00:00".to_owned())
    );
}

#[rstest]
#[case::infinite(f64::INFINITY)]
#[case::nan(f64::NAN)]
#[case::huge(1e300)]
#[tokio::test]
async fn an_expiry_no_calendar_can_add_is_an_error(#[case] seconds: f64) {
    let dual = Arc::new(FakeCache {
        spend: None,
        spend_fails: false,
        expiry: Some(seconds),
        keys: Mutex::new(Vec::new()),
    });
    let view = ProviderBudgetView {
        provider_budget_config: Some(config(&[("openai", Some("1d"))])),
        redis: None,
        dual: Arc::clone(&dual),
    };
    let error = view
        .provider_budget_reset_at("openai", instant("2026-11-19T12:00:00Z"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProviderBudgetViewError::Expiry { key } if key == "provider_spend:openai:1d"
    ));
}
