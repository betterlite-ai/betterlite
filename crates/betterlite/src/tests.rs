#![cfg(test)]

use super::{config_error_category, log_config_error};
use betterlite_config::loader::ConfigError;
use betterlite_env::EnvError;
use std::path::Path;

#[test]
fn unsupported_variant_categorizes_as_unsupported_key() {
    let error = ConfigError::Unsupported {
        paths: vec!["general_settings.unknown".to_owned()],
    };
    assert_eq!(config_error_category(&error), "unsupported_key");
}

#[test]
fn missing_environment_variant_categorizes_as_missing_environment() {
    let error = ConfigError::MissingEnvironment {
        variable: "MISSING".to_owned(),
        path: "model.api_key".to_owned(),
    };
    assert_eq!(config_error_category(&error), "missing_environment");
}

#[test]
fn invalid_float_variant_categorizes_as_invalid_float() {
    let error = ConfigError::InvalidFloat {
        path: "model.temperature".to_owned(),
        source: "not-a-float".parse::<f64>().unwrap_err(),
    };
    assert_eq!(config_error_category(&error), "invalid_float");
}

#[test]
fn invalid_integer_variant_categorizes_as_invalid_integer() {
    let error = ConfigError::InvalidInteger {
        path: "model.max_tokens".to_owned(),
        source: "not-an-integer".parse::<i64>().unwrap_err(),
    };
    assert_eq!(config_error_category(&error), "invalid_integer");
}

#[test]
fn invalid_scalar_variant_categorizes_as_invalid_scalar() {
    let error = ConfigError::InvalidScalar {
        path: "model.stream".to_owned(),
        value: true,
    };
    assert_eq!(config_error_category(&error), "invalid_scalar");
}

#[test]
fn environment_variant_categorizes_as_environment() {
    let error = ConfigError::Environment {
        source: EnvError::Bool {
            name: "DISABLE_AIOHTTP_TRUST_ENV".to_owned(),
        },
    };
    assert_eq!(config_error_category(&error), "environment");
}

#[test]
fn log_config_error_covers_the_generic_arm_without_panicking() {
    log_config_error(
        &ConfigError::Environment {
            source: EnvError::Number {
                name: "REDIS_CONNECTION_POOL_SIZE".to_owned(),
            },
        },
        Path::new("config.yaml"),
    );
}
