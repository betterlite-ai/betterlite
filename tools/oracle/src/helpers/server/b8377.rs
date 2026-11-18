use std::collections::BTreeMap;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_server::coord_redis_boot::build::{env_has_redis_target, usage_family};
use betterlite_server::coord_redis_boot::env_refs::{
    EnvironmentSecrets, ResolveContext, resolve_env_refs,
};
use betterlite_server::coord_redis_boot::error::CoordRedisError;
use betterlite_server::coord_redis_boot::init::config_coordination_params;
use betterlite_server::proxy_request_contracts::coordination_redis::CoordinationRedisParams;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    EnvRefs {
        params: Value,
        environment: BTreeMap<String, String>,
    },
    UsageFamily {
        params: Value,
        environment: BTreeMap<String, String>,
    },
    EnvHasTarget {
        environment: BTreeMap<String, String>,
    },
    ConfigError {
        config: Value,
        environment: BTreeMap<String, String>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::EnvRefs {
                params,
                environment,
            } => resolved_env_refs(params, environment),
            Input::UsageFamily {
                params,
                environment,
            } => family_of(params, environment),
            Input::EnvHasTarget { environment } => {
                Ok(Value::Bool(env_has_redis_target(environment)))
            }
            Input::ConfigError {
                config,
                environment,
            } => config_error_kind(config, environment),
        }
    }
}

// litellm hands the builder the validated model with its None fields dropped.
fn resolved_env_refs(params: &Value, environment: &BTreeMap<String, String>) -> HelperResult {
    let context = ResolveContext {
        secrets: &EnvironmentSecrets,
        env: environment,
    };
    let resolved = resolve_env_refs(params, &context).or_value_error()?;
    let model = CoordinationRedisParams::from_value(&resolved).or_value_error()?;
    match serde_json::to_value(&model).or_value_error()? {
        Value::Object(mut fields) => {
            fields.retain(|_name, value| !value.is_null());
            Ok(Value::Object(fields))
        }
        other => Ok(other),
    }
}

fn family_of(params: &Value, environment: &BTreeMap<String, String>) -> HelperResult {
    let model = CoordinationRedisParams::from_value(params).or_value_error()?;
    let family = usage_family(&model, environment).or_value_error()?;
    let name = match family {
        RedisCacheFamily::Cluster => "cluster",
        RedisCacheFamily::Standalone => "standalone",
    };
    Ok(Value::String(name.to_owned()))
}

fn config_error_kind(config: &Value, environment: &BTreeMap<String, String>) -> HelperResult {
    let context = ResolveContext {
        secrets: &EnvironmentSecrets,
        env: environment,
    };
    let kind = match config_coordination_params(config, &context) {
        Ok(_) => "none",
        Err(CoordRedisError::NotMapping) => "not_mapping",
        Err(CoordRedisError::MissingTarget) => "missing_target",
        Err(_) => "other",
    };
    Ok(Value::String(kind.to_owned()))
}

mod tests;
