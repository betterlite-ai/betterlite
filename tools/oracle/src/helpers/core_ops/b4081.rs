use std::collections::BTreeMap;

use betterlite_core::shadow_evidence::{chat_text, cost, judge_reply, responses_text, routing};
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    response: Json,
    metadata: Json,
    routed_model: String,
    #[serde(default)]
    prices: BTreeMap<String, Price>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    ChatText,
    ResponsesText,
    CallCost,
    JudgeShape,
    Routing,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Price {
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    prompt: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    completion: Decimal,
}

// The price map the cases carry, mirroring what the bundled LiteLLM cost map answers
struct CasePrices<'prices>(&'prices BTreeMap<String, Price>);

impl cost::PriceMap for CasePrices<'_> {
    fn completion_cost(&self, response: &Json) -> Option<Decimal> {
        let Some(Json::Text(model)) = chat_text::field(response, "model") else {
            return None;
        };
        let price = self.0.get(model)?;
        let usage = chat_text::field(response, "usage")?;
        let prompt = token_count(usage, "prompt_tokens")?;
        let completion = token_count(usage, "completion_tokens")?;
        let prompt_charge = price.prompt.checked_mul(Decimal::from(prompt))?;
        let completion_charge = price.completion.checked_mul(Decimal::from(completion))?;
        prompt_charge.checked_add(completion_charge)
    }
}

fn token_count(usage: &Json, name: &str) -> Option<u64> {
    match chat_text::field(usage, name) {
        Some(Json::Number(number)) => number.as_u64(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let response = &input.response;
        let metadata = &input.metadata;
        match input.operation {
            Operation::ChatText => Ok(json!({
                "text": chat_text::final_text(response),
                "finish_reason": chat_text::finish_reason(response),
                "empty_error": chat_text::empty_reply_error(response, &input.routed_model),
            })),
            Operation::ResponsesText => Ok(json!({"text": responses_text::final_text(response)})),
            Operation::CallCost => {
                let prices = CasePrices(&input.prices);
                Ok(json!({
                    "cost": cost::call_cost(response, &prices).normalize().to_string(),
                }))
            }
            Operation::JudgeShape => Ok(json!({"shape": judge_reply::shape(response)})),
            Operation::Routing => Ok(json!({
                "decision": Json::Object(routing::decision(metadata).cloned().unwrap_or_default()),
                "tier": routing::routed_tier(metadata),
                "classifier_cost": routing::classifier_cost(metadata).normalize().to_string(),
            })),
        }
    }
}
