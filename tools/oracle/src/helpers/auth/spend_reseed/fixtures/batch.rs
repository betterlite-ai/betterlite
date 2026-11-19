use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_auth::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use betterlite_auth::spend_reseed::cache::ActiveSpendBatch;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::Value;

use super::fakes::amount_json;
use super::input::{Answer, Word};

// LiteLLM opens a spend counter batch only for a Redis-backed request that binds counters
pub(crate) struct FakeBatch {
    present: bool,
    answer: Option<Answer>,
    recorded: Mutex<Option<Decimal>>,
}

impl FakeBatch {
    pub(crate) const fn new(present: bool, answer: Option<Answer>) -> Self {
        Self {
            present,
            answer,
            recorded: Mutex::new(None),
        }
    }

    pub(crate) fn after(&self) -> Value {
        if !self.present {
            return Value::Null;
        }
        let recorded = *self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(amount) = recorded {
            return amount_json(amount);
        }
        self.scripted_answer()
    }

    fn scripted_answer(&self) -> Value {
        match &self.answer {
            Some(Answer::Amount(value)) => {
                Decimal::from_f64(*value).map_or(Value::Null, amount_json)
            }
            Some(Answer::Word(Word::Miss | Word::Fail)) | None => Value::Null,
        }
    }
}

impl ActiveSpendBatch for FakeBatch {
    fn read(
        &self,
        _counter_key: &str,
    ) -> impl Future<Output = Result<Option<CounterValue>, SpendCounterBatchError>> + Send {
        ready(batch_value(self.answer.as_ref()))
    }

    fn record(&self, _counter_key: &str, value: Decimal) {
        if !self.present {
            return;
        }
        *self.recorded.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
    }
}

fn batch_value(answer: Option<&Answer>) -> Result<Option<CounterValue>, SpendCounterBatchError> {
    match answer {
        Some(Answer::Amount(value)) => {
            let amount = Decimal::from_f64(*value).ok_or(SpendCounterBatchError::Cancelled)?;
            Ok(Some(CounterValue::Amount(amount)))
        }
        Some(Answer::Word(Word::Miss)) => Ok(Some(CounterValue::Missing)),
        Some(Answer::Word(Word::Fail)) | None => Ok(None),
    }
}
