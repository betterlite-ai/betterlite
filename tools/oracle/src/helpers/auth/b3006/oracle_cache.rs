use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_auth::model_budget_limiter::cache::ModelBudgetCache;
use betterlite_auth::model_budget_values::amount_text::amount_from_text;
use betterlite_auth::model_budget_values::spend_reads::CachedSpend;
use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use thiserror::Error;

use crate::helpers::HelperError;

#[derive(Debug, Error)]
#[error("the oracle cache never fails an increment")]
pub(super) struct CacheFailure;

pub(super) struct OracleCache {
    values: Mutex<BTreeMap<String, CachedSpend>>,
}

impl OracleCache {
    pub(super) fn new(memory: &BTreeMap<String, JsonData>) -> Result<Self, Box<HelperError>> {
        let mut values = BTreeMap::new();
        for (key, raw) in memory {
            match raw {
                JsonData::Null => {}
                JsonData::Number(number) => {
                    let Some(amount) = amount_from_text(&number.to_string()) else {
                        return Err(invalid(format!("counter {key} is not a decimal number")));
                    };
                    values.insert(key.clone(), CachedSpend::Number(amount));
                }
                JsonData::String(text) => {
                    values.insert(key.clone(), CachedSpend::Text(text.clone()));
                }
                JsonData::Bool(_) | JsonData::Array(_) | JsonData::Object(_) => {
                    values.insert(key.clone(), CachedSpend::Unreadable);
                }
            }
        }
        Ok(Self {
            values: Mutex::new(values),
        })
    }

    pub(super) fn state(&self) -> (Value, Value) {
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        let mut spends = BTreeMap::new();
        let mut markers = BTreeMap::new();
        for (key, value) in values.iter() {
            let CachedSpend::Number(amount) = value else {
                continue;
            };
            if key.contains(":batch:") {
                markers.insert(key.clone(), json!(amount.as_f64()));
                continue;
            }
            if spends_key(key) {
                spends.insert(key.clone(), json!(amount.as_f64()));
            }
        }
        drop(values);
        (json!(spends), json!(markers))
    }

    fn amount(&self, key: &str) -> Decimal {
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        match values.get(key) {
            Some(CachedSpend::Number(amount)) => *amount,
            Some(CachedSpend::Text(text)) => amount_from_text(text.trim()).unwrap_or(Decimal::ZERO),
            Some(CachedSpend::Unreadable) | None => Decimal::ZERO,
        }
    }

    fn add(&self, key: &str, cost: Decimal) {
        let next = self.amount(key).checked_add(cost).unwrap_or(Decimal::MAX);
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), CachedSpend::Number(next));
    }

    fn cached(&self, key: &str) -> Option<CachedSpend> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }
}

impl ModelBudgetCache for OracleCache {
    type Error = CacheFailure;

    fn redis_configured(&self) -> bool {
        false
    }

    fn redis_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send {
        ready(self.cached(key))
    }

    fn memory_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send {
        ready(self.cached(key))
    }

    fn increment_counter(
        &self,
        key: &str,
        _ttl: Duration,
    ) -> impl Future<Output = Option<Decimal>> + Send {
        self.add(key, Decimal::ONE);
        ready(Some(self.amount(key)))
    }

    fn increment_spend(
        &self,
        _duration: Duration,
        spend_key: &str,
        _start_time_key: &str,
        cost: Decimal,
    ) -> impl Future<Output = Result<(), CacheFailure>> + Send {
        self.add(spend_key, cost);
        ready(Ok(()))
    }

    fn flush_increments(&self) -> impl Future<Output = ()> + Send {
        ready(())
    }
}

fn spends_key(key: &str) -> bool {
    key.starts_with("virtual_key_spend:")
        || key.starts_with("user_model_spend:")
        || key.starts_with("end_user_model_spend:")
        || key.starts_with("team_model_spend:")
}

pub(super) fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
