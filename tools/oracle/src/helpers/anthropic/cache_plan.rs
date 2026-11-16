use std::collections::BTreeMap;

use betterlite_anthropic::cache_plan::{parse_cache_plan, parse_prompt};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use sha2::{Digest as _, Sha256};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    CachePlan {
        body: String,
    },
    Prompt {
        body: String,
    },
    CacheScope {
        caller_key_hash: String,
        deployment_id: String,
        provider_key: String,
        model: String,
        anthropic_version: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let value = match input {
            Input::CachePlan { body } => match parse_cache_plan(body) {
                Ok(plan) => serde_json::to_value(plan),
                Err(reason) => serde_json::to_value(BTreeMap::from([("unsupported", reason)])),
            },
            Input::Prompt { body } => {
                serde_json::to_value(BTreeMap::from([("prompt", parse_prompt(body))]))
            }
            Input::CacheScope {
                caller_key_hash,
                deployment_id,
                provider_key,
                model,
                anthropic_version,
            } => serde_json::to_vec(&(
                caller_key_hash,
                deployment_id,
                provider_key,
                model,
                anthropic_version,
            ))
            .map(|text| serde_json::Value::String(hex::encode(Sha256::digest(text)))),
        };
        value.map_err(|error| {
            Box::new(HelperError {
                class: "CachePlanJson".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
