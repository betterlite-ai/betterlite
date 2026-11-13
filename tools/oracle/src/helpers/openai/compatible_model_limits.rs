use betterlite_openai::compatible_models::decode_compatible_model_list;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    model: String,
    body: String,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let limits = input.calls.iter().map(|call| {
            let found = decode_compatible_model_list(call.body.as_bytes())
                .ok()
                .and_then(|list| list.data.into_iter().find(|card| card.id == call.model))
                .map(|card| card.token_limits());
            json!(found.unwrap_or_default())
        });
        Ok(Value::Array(limits.collect()))
    }
}

mod tests;
