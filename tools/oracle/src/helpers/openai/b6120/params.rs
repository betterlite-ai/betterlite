use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_openai::perplexity_chat::{CapabilityLookup, supported_openai_params};

use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Steps {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    model: String,
    reasoning: Lookup,
    web_search: Lookup,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Lookup {
    False,
    True,
    Error,
}

struct Fixture {
    reasoning: Lookup,
    web_search: Lookup,
}

impl CapabilityLookup for Fixture {
    type Error = std::io::Error;

    fn supports_reasoning(&self, _model: &str) -> Result<bool, Self::Error> {
        answer(self.reasoning, "reasoning")
    }

    fn supports_web_search(&self, _model: &str) -> Result<bool, Self::Error> {
        answer(self.web_search, "web search")
    }
}

fn answer(lookup: Lookup, name: &str) -> Result<bool, std::io::Error> {
    match lookup {
        Lookup::True => Ok(true),
        Lookup::False => Ok(false),
        Lookup::Error => Err(std::io::Error::other(format!(
            "the {name} capability lookup failed"
        ))),
    }
}

pub(super) fn run(steps: &Steps) -> HelperResult {
    let outcomes: Vec<Value> = steps
        .steps
        .iter()
        .map(|step| {
            let fixture = Fixture {
                reasoning: step.reasoning,
                web_search: step.web_search,
            };
            json!({"supported": supported_openai_params(&fixture, &step.model)})
        })
        .collect();
    Ok(Value::Array(outcomes))
}
