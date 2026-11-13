use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::rate_limiter_settings;
use betterlite_rate_limit::limiter_records::{
    RateLimitType, RateLimitedModel, create_rate_limit_keys, sibling_counter_keys,
};
use serde::Deserialize;

use crate::helpers::OrHelperError as _;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    CounterKeys {
        key: String,
        value: String,
        rate_limit_type: RateLimitType,
    },
    SiblingCounterKeys {
        window_key: String,
    },
    ModelLimit {
        requested: String,
        group: String,
        limits: Option<BTreeMap<String, i64>>,
    },
    FailClosedFlag {
        config: serde_json::Value,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::CounterKeys {
                key,
                value,
                rate_limit_type,
            } => Ok(serde_json::json!({
                "counter_key": create_rate_limit_keys(key, value, *rate_limit_type),
            })),
            Input::SiblingCounterKeys { window_key } => {
                let (requests_key, tokens_key) = sibling_counter_keys(window_key);
                Ok(serde_json::json!({
                    "requests_key": requests_key,
                    "tokens_key": tokens_key,
                }))
            }
            Input::ModelLimit {
                requested,
                group,
                limits,
            } => {
                let model = RateLimitedModel {
                    requested: requested.clone(),
                    group: group.clone(),
                };
                Ok(serde_json::json!({ "limit": model.limit_in(limits.as_ref()) }))
            }
            Input::FailClosedFlag { config } => {
                let raw: rate_limiter_settings::Raw = serde_json::from_value(config.clone())?;
                let settings = raw
                    .convert(&BTreeMap::<String, String>::new())
                    .or_config_error()?;
                Ok(serde_json::json!({
                    "enabled": settings.fail_closed,
                }))
            }
        }
    }
}
