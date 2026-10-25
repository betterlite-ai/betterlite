use std::collections::BTreeMap;

use betterlite_anthropic::cache_plan::{cache_scope, parse_cache_plan, parse_prompt};

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

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
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
            } => cache_scope(
                caller_key_hash,
                deployment_id,
                provider_key,
                model,
                anthropic_version,
            )
            .map(serde_json::Value::String),
        };
        value.map_err(|error| {
            Box::new(crate::helpers::HelperError {
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
