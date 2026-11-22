use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::model_budget_limiter::ModelBudgetLimiter;
use betterlite_auth::model_budget_limiter::cache::ModelBudgetCache;
use betterlite_auth::model_budget_limiter::clock::BudgetClock;
use betterlite_auth::model_budget_limiter::enforcement::BudgetSubject;
use betterlite_auth::model_budget_values::amount_text::amount_from_text;
use betterlite_auth::model_budget_values::candidates::BedrockCatalog;
use betterlite_auth::model_budget_values::spend_reads::CachedSpend;
use betterlite_core::api::summary_model_limits::budget::SummaryModelBudgets;
use betterlite_core::api::summary_model_limits::fields::BudgetMap;
use betterlite_core::duration::seconds::duration_in_seconds;
use betterlite_error::error::{ErrorClass, ErrorContext};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

// LiteLLM's DualCache.async_get_cache raises on these keys, and B3006's cache can only miss, so the miss is flagged
pub(super) struct ScriptedSpend {
    spend: BTreeMap<String, Decimal>,
    offline: BTreeSet<String>,
    reads: Mutex<Vec<String>>,
    tripped: AtomicBool,
}

impl ScriptedSpend {
    pub(super) fn reads(&self) -> Vec<String> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted spend cache never fails a write")]
pub(super) struct NeverFails;

impl ModelBudgetCache for ScriptedSpend {
    type Error = NeverFails;

    fn redis_configured(&self) -> bool {
        false
    }

    fn redis_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send {
        self.memory_spend(key)
    }

    fn memory_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        if self.offline.contains(key) {
            self.tripped.store(true, Ordering::SeqCst);
        }
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

struct ZoneDurations(TimeZoneDatabase);

impl DurationParser for ZoneDurations {
    fn seconds(&self, spec: &str, now: DateTime<Utc>) -> Option<i64> {
        duration_in_seconds(spec, now, None, &self.0)
            .ok()
            .map(|parsed| parsed.seconds)
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

// a fresh LiteLLM process is inside its first budget window, so the legacy carry is live
struct JustStarted;

impl BudgetClock for JustStarted {
    fn now(&self) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }

    fn uptime(&self) -> Duration {
        Duration::ZERO
    }
}

pub(super) struct OracleBudgets {
    pub(super) limiter: ModelBudgetLimiter<ScriptedSpend>,
}

impl SummaryModelBudgets for OracleBudgets {
    async fn within_model_budget(
        &self,
        subject: &BudgetSubject<'_>,
        model_max_budget: &BudgetMap,
        model: &str,
    ) -> Result<(), ErrorClass> {
        let checked = self
            .limiter
            .within_model_budget(subject, model_max_budget, model)
            .await;
        if !self.limiter.cache.tripped.swap(false, Ordering::SeqCst) {
            return checked;
        }
        Err(ErrorClass::Unclassified(Box::new(ErrorContext {
            message: "offline".to_owned(),
            model: None,
            provider: None,
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        })))
    }
}

pub(super) fn spend_limiter(
    spend: &BTreeMap<String, serde_json::Number>,
    offline: &[String],
) -> Result<OracleBudgets, Box<HelperError>> {
    let mut amounts = BTreeMap::new();
    for (key, number) in spend {
        let amount = amount_from_text(&number.to_string())
            .ok_or_else(|| format!("spend {key} is not a decimal number"))
            .or_value_error()?;
        amounts.insert(key.clone(), amount);
    }
    let cache = ScriptedSpend {
        spend: amounts,
        offline: offline.iter().cloned().collect(),
        reads: Mutex::new(Vec::new()),
        tripped: AtomicBool::new(false),
    };
    Ok(OracleBudgets {
        limiter: ModelBudgetLimiter {
            cache,
            durations: Box::new(ZoneDurations(TimeZoneDatabase::from_env())),
            catalog: Box::new(PlainModels),
            clock: Box::new(JustStarted),
            budget_exceeded_status: 422,
        },
    })
}
