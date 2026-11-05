use std::collections::HashMap;

use betterlite_context::bedrock_model::get_bedrock_base_model;
use betterlite_models::model_catalog_keys::{
    EntryProvider, ModelCostKeys, PotentialModelNames, check_provider_match,
    get_model_info_from_model_cost, strip_model_name,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model_cost: Vec<String>,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    StripModelName {
        model: String,
        custom_llm_provider: Option<String>,
    },
    GetKey {
        key: String,
    },
    Remove {
        key: String,
    },
    Add {
        key: String,
    },
    Index,
    Generation,
    Invalidate,
    CheckProviderMatch {
        litellm_provider: Option<Value>,
        custom_llm_provider: Option<String>,
    },
    Direct {
        key: String,
    },
    Potential {
        names: PotentialModelNames,
    },
}

pub(crate) struct Op;

struct Session {
    model_cost: IndexMap<String, usize>,
    keys: ModelCostKeys,
    full_cache: HashMap<&'static str, usize>,
    helper_cache: HashMap<&'static str, usize>,
}

fn provider_match(litellm_provider: Option<&Value>, custom_llm_provider: Option<&str>) -> Value {
    let entry = match litellm_provider {
        None | Some(Value::Null) => EntryProvider::Unconstrained,
        Some(Value::String(text)) => EntryProvider::Text(text),
        Some(_) => EntryProvider::NotText,
    };
    check_provider_match(entry, custom_llm_provider)
        .map_or_else(|_| json!({"error": "raised"}), |matched| json!(matched))
}

impl Session {
    fn invalidate(&mut self) -> Value {
        let before = [self.full_cache.len(), self.helper_cache.len()];
        self.keys
            .invalidate(&mut self.full_cache, &mut self.helper_cache);
        let after = [self.full_cache.len(), self.helper_cache.len()];
        json!({"before": before, "after": after, "index": self.keys.lowercase_index, "generation": self.keys.generation})
    }

    fn apply(&mut self, step: &Step) -> Value {
        match step {
            Step::StripModelName {
                model,
                custom_llm_provider,
            } => {
                json!(strip_model_name(
                    model,
                    custom_llm_provider.as_deref(),
                    get_bedrock_base_model
                ))
            }
            Step::GetKey { key } => json!(self.keys.get_model_cost_key(&self.model_cost, key)),
            Step::Remove { key } => {
                self.model_cost.shift_remove(key);
                Value::Null
            }
            Step::Add { key } => {
                self.model_cost.insert(key.clone(), self.model_cost.len());
                Value::Null
            }
            Step::Index => json!(self.keys.lowercase_index),
            Step::Generation => json!(self.keys.generation),
            Step::Invalidate => self.invalidate(),
            Step::CheckProviderMatch {
                litellm_provider,
                custom_llm_provider,
            } => provider_match(litellm_provider.as_ref(), custom_llm_provider.as_deref()),
            Step::Direct { key } => get_model_info_from_model_cost(&self.model_cost, key)
                .map_or_else(
                    |_| json!({"error": "raised"}),
                    |position| json!({"position": position}),
                ),
            Step::Potential { names } => {
                json!(self.keys.has_potential_model_name(&self.model_cost, names))
            }
        }
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut session = Session {
            model_cost: input.model_cost.iter().cloned().zip(0..).collect(),
            keys: ModelCostKeys::default(),
            full_cache: HashMap::from([("full", 1)]),
            helper_cache: HashMap::from([("helper", 1)]),
        };
        Ok(json!(
            input
                .steps
                .iter()
                .map(|step| session.apply(step))
                .collect::<Vec<_>>()
        ))
    }
}

mod tests;
