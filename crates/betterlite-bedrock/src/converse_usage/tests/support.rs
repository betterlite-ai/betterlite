use betterlite_models::decisions::Json;

use crate::converse_usage::transform::ReasoningTokenCounter;
use crate::converse_usage::wire::ConverseTokenUsage;

#[derive(Debug, thiserror::Error)]
#[error("the test counter is unavailable")]
pub(super) struct CounterFailed;

pub(super) struct FixtureCounter(pub(super) Option<i64>);

impl ReasoningTokenCounter for FixtureCounter {
    type Failure = CounterFailed;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<i64, Self::Failure> {
        self.0.ok_or(CounterFailed)
    }
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(super) fn usage(source: &str) -> ConverseTokenUsage {
    ConverseTokenUsage::decode(&json(source)).unwrap()
}

pub(super) fn rendered<Value>(value: &Value) -> Json
where
    Value: serde::Serialize,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}
