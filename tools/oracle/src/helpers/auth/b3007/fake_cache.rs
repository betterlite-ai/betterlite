use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_auth::model_budget_values::amount_text::amount_from_text;
use betterlite_auth::model_budget_values::spend_reads::CachedSpend;
use betterlite_auth::session_budget::cache::SessionBudgetCache;
use betterlite_oracle_helper_api::error::HelperError;
use rust_decimal::Decimal;
use thiserror::Error;

use super::input::RedisMode;

#[derive(Debug, Error)]
#[error("the fake Redis failed on purpose")]
pub(super) struct FakeFailure;

pub(super) struct FakeCache {
    mode: RedisMode,
    pub(super) memory: Mutex<BTreeMap<String, CachedSpend>>,
    pub(super) memory_ttls: Mutex<BTreeMap<String, u64>>,
    pub(super) redis: Mutex<BTreeMap<String, CachedSpend>>,
    pub(super) redis_ttls: Mutex<BTreeMap<String, u64>>,
}

impl FakeCache {
    pub(super) fn new(
        mode: RedisMode,
        memory: &BTreeMap<String, serde_json::Number>,
        redis_values: &BTreeMap<String, String>,
    ) -> Result<Self, Box<HelperError>> {
        let mut seeded = BTreeMap::new();
        for (key, number) in memory {
            let Some(amount) = amount_from_text(&number.to_string()) else {
                return Err(invalid(format!("memory value for {key} is not a number")));
            };
            seeded.insert(key.clone(), CachedSpend::Number(amount));
        }
        let redis = redis_values
            .iter()
            .map(|(key, text)| (key.clone(), CachedSpend::Text(text.clone())))
            .collect();
        Ok(Self {
            mode,
            memory: Mutex::new(seeded),
            memory_ttls: Mutex::new(BTreeMap::new()),
            redis: Mutex::new(redis),
            redis_ttls: Mutex::new(BTreeMap::new()),
        })
    }

    fn read(store: &Mutex<BTreeMap<String, CachedSpend>>, key: &str) -> Option<CachedSpend> {
        store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn add(&self, key: &str, amount: Decimal, ttl: Duration) -> Decimal {
        let mut values = self.redis.lock().unwrap_or_else(PoisonError::into_inner);
        let existed = values.contains_key(key);
        let current = values.get(key).and_then(amount_of).unwrap_or(Decimal::ZERO);
        let next = current.checked_add(amount).unwrap_or(Decimal::MAX);
        values.insert(key.to_owned(), CachedSpend::Number(next));
        drop(values);
        if !existed {
            self.redis_ttls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key.to_owned(), ttl.as_secs());
        }
        next
    }
}

pub(super) fn state(
    values: &Mutex<BTreeMap<String, CachedSpend>>,
    ttls: &Mutex<BTreeMap<String, u64>>,
) -> (BTreeMap<String, f64>, BTreeMap<String, u64>) {
    let spent = {
        let locked = values.lock().unwrap_or_else(PoisonError::into_inner);
        amounts(&locked)
    };
    let lifetimes = ttls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (spent, lifetimes)
}

impl SessionBudgetCache for FakeCache {
    type Error = FakeFailure;
    type Script = ();

    fn redis_configured(&self) -> bool {
        self.mode != RedisMode::Absent
    }

    fn register_increment_script(&self, _script: &str) -> Option<()> {
        Some(())
    }

    fn redis_spend(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<CachedSpend>, FakeFailure>> + Send {
        if self.mode == RedisMode::ReadFails {
            return ready(Err(FakeFailure));
        }
        ready(Ok(Self::read(&self.redis, key)))
    }

    fn run_increment_script(
        &self,
        _script: &(),
        key: &str,
        amount: Decimal,
        ttl: Duration,
    ) -> impl Future<Output = Result<Decimal, FakeFailure>> + Send {
        if self.mode == RedisMode::ScriptFails {
            return ready(Err(FakeFailure));
        }
        ready(Ok(self.add(key, amount, ttl)))
    }

    fn memory_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send {
        ready(Self::read(&self.memory, key))
    }

    fn memory_set_spend(
        &self,
        key: &str,
        value: CachedSpend,
        ttl: Duration,
    ) -> impl Future<Output = ()> + Send {
        self.memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
        self.memory_ttls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), ttl.as_secs());
        ready(())
    }
}

fn amounts(values: &BTreeMap<String, CachedSpend>) -> BTreeMap<String, f64> {
    let mut amounts = BTreeMap::new();
    for (key, value) in values {
        let Some(amount) = amount_of(value) else {
            continue;
        };
        amounts.insert(key.clone(), amount.as_f64());
    }
    amounts
}

fn amount_of(value: &CachedSpend) -> Option<Decimal> {
    match value {
        CachedSpend::Number(amount) => Some(*amount),
        CachedSpend::Text(text) => amount_from_text(text.trim()),
        CachedSpend::Unreadable => None,
    }
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
