use std::collections::BTreeMap;

use betterlite_redis::redis_connection_settings::model::settings::{ClientKind, RedisOverrides};
use betterlite_redis::redis_connection_settings::resolve::resolve_redis_kwargs;
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClientKindWire {
    #[default]
    Sync,
    Async,
}

impl From<ClientKindWire> for ClientKind {
    fn from(value: ClientKindWire) -> Self {
        match value {
            ClientKindWire::Sync => Self::Sync,
            ClientKindWire::Async => Self::Async,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ResolveRedisKwargs {
        #[serde(default)]
        environment: BTreeMap<String, String>,
        #[serde(default)]
        client: ClientKindWire,
        overrides: RedisOverrides,
    },
}

#[derive(Debug, Error)]
enum Failure {
    #[error(transparent)]
    Settings(#[from] betterlite_redis::redis_connection_settings::error::RedisSettingsError),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let Input::ResolveRedisKwargs {
            environment,
            client,
            overrides,
        } = input;
        let settings =
            resolve_redis_kwargs(overrides.clone(), ClientKind::from(*client), environment)
                .map_err(Failure::from)?;
        let uses_tls = betterlite_redis::redis_connection_settings::url_parts::uses_tls(
            settings
                .startup_nodes
                .as_ref()
                .is_some_and(|nodes| !nodes.is_empty()),
            settings.ssl,
            settings.url.as_ref().map(ExposeSecret::expose_secret),
        );
        Ok(json!({
            "host": settings.host,
            "port": settings.port,
            "db": settings.db,
            "username": settings.username,
            "password": settings.password.as_ref().map(ExposeSecret::expose_secret),
            "url": settings.url.as_ref().map(ExposeSecret::expose_secret),
            "ssl": settings.ssl,
            "socket_timeout": settings.socket_timeout,
            "socket_connect_timeout": settings.socket_connect_timeout,
            "socket_keepalive": settings.socket_keepalive,
            "max_connections": settings.max_connections,
            "health_check_interval": settings.health_check_interval,
            "credential_provider": settings.credential_provider,
            "startup_nodes": settings.startup_nodes,
            "sentinel_nodes": settings.sentinel_nodes,
            "sentinel_password": settings.sentinel_password.as_ref().map(ExposeSecret::expose_secret),
            "service_name": settings.service_name,
            "ssl_password": settings.ssl_password.as_ref().map(ExposeSecret::expose_secret),
            "passthrough": settings.passthrough,
            "uses_tls": uses_tls,
        }))
    }
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        // Maps each RedisSettingsError variant to the litellm exception class it mirrors (see docs/deviations/B4321.yaml for InvalidUrl).
        let class = match &failure {
            Failure::Settings(error) => match error {
                betterlite_redis::redis_connection_settings::error::RedisSettingsError::MissingTarget
                | betterlite_redis::redis_connection_settings::error::RedisSettingsError::AwsIamRequiresTls
                | betterlite_redis::redis_connection_settings::error::RedisSettingsError::AwsIamMissingSettings { .. } => {
                    "ValueError"
                }
                betterlite_redis::redis_connection_settings::error::RedisSettingsError::InvalidClusterNodesJson { .. }
                | betterlite_redis::redis_connection_settings::error::RedisSettingsError::InvalidSentinelNodesJson { .. } => {
                    "JSONDecodeError"
                }
                betterlite_redis::redis_connection_settings::error::RedisSettingsError::InvalidUrl { .. }
                | betterlite_redis::redis_connection_settings::error::RedisSettingsError::UrlCannotStripUserinfo => {
                    "RuntimeError"
                }
                // Url/Cluster/Sentinel targets silently drop an unrecognized name in litellm; only a bare Direct connection forwards it straight into redis.Redis.__init__, which raises TypeError for an unexpected keyword argument.
                betterlite_redis::redis_connection_settings::error::RedisSettingsError::UnknownOption { .. } => {
                    "TypeError"
                }
            },
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}
