use std::str::FromStr as _;

use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

// The spend fields both relay handlers write: the model, the provider that billed it and its cost.
#[derive(Clone, Debug)]
pub struct MediaSpend {
    pub model: String,
    pub custom_llm_provider: &'static str,
    // A registry lookup that found no price leaves the cost absent, never zero.
    pub response_cost: Option<Decimal>,
}

impl MediaSpend {
    // The logging object's model_call_details and the returned kwargs carry the same three keys.
    #[must_use]
    pub fn entries(&self) -> IndexMap<String, Json> {
        let mut entries = IndexMap::new();
        entries.insert("model".to_owned(), Json::Text(self.model.clone()));
        entries.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(self.custom_llm_provider.to_owned()),
        );
        entries.insert(
            "response_cost".to_owned(),
            self.response_cost.map_or(Json::Null, decimal_json),
        );
        entries
    }

    #[must_use]
    pub fn kwargs(&self, requested: &PassThroughLoggingKwargs) -> PassThroughLoggingKwargs {
        let mut kwargs = requested.clone();
        for (key, value) in self.entries() {
            kwargs.entries.insert(key, value);
        }
        kwargs
    }
}

// A Decimal's plain text is always a JSON number; Null keeps an impossible one out of the log.
#[must_use]
pub fn decimal_json(cost: Decimal) -> Json {
    match Number::from_str(&cost.normalize().to_string()) {
        Ok(number) => Json::Number(number),
        Err(_error) => Json::Null,
    }
}
