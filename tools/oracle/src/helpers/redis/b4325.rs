use std::collections::BTreeMap;

use betterlite_redis::redis_client_factories::cluster::ClusterClient;
use betterlite_redis::redis_client_reuse::cache_key::async_client_cache_key;
use betterlite_redis::redis_client_reuse::error::ClientReuseError;
use betterlite_redis::redis_client_reuse::pubsub::cluster_node_connection;
use betterlite_redis::redis_connection_settings::model::nodes::ClusterNodeConfig;
use betterlite_redis::redis_connection_settings::model::settings::{ClientKind, RedisOverrides};
use betterlite_redis::redis_connection_settings::resolve::resolve_redis_kwargs;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NodeInput {
    host: String,
    port: i64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ClusterPubsubNode {
        #[serde(default)]
        startup_nodes: Vec<NodeInput>,
        username: Option<String>,
        password: Option<String>,
        ssl: Option<bool>,
        socket_timeout: Option<f64>,
    },
    ClientKeySame {
        left: Box<RedisOverrides>,
        right: Box<RedisOverrides>,
    },
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn settings_key(overrides: &RedisOverrides) -> Result<String, Box<HelperError>> {
    let environment = BTreeMap::<String, String>::new();
    let settings = resolve_redis_kwargs(overrides.clone(), ClientKind::Async, &environment)
        .map_err(|error| failure("ValueError", error.to_string()))?;
    Ok(async_client_cache_key(&settings))
}

fn cluster_pubsub_output(
    startup_nodes: &[NodeInput],
    username: Option<&str>,
    password: Option<&str>,
    ssl: Option<bool>,
    socket_timeout: Option<f64>,
) -> HelperResult {
    let record = ClusterClient {
        startup_nodes: startup_nodes
            .iter()
            .map(|node| ClusterNodeConfig {
                host: node.host.clone(),
                port: node.port,
            })
            .collect(),
        username: username.map(str::to_owned),
        password: password.map(|text| SecretString::from(text.to_owned())),
        ssl,
        ssl_password: None,
        socket_timeout,
        socket_connect_timeout: None,
        max_connections: None,
        health_check_interval: None,
        socket_keepalive: None,
        credential_provider: false,
        credential_provider_settings: None,
        passthrough: BTreeMap::new(),
        isolation: None,
    };
    let fields = cluster_node_connection(&record)
        .map_err(|error: ClientReuseError| failure("ValueError", error.to_string()))?;
    Ok(json!({
        "host": fields.host,
        "port": fields.port,
        "username": fields.username,
        "has_password": fields.password.is_some(),
        "ssl": fields.ssl.unwrap_or(false),
        "socket_timeout": fields.socket_timeout,
    }))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ClusterPubsubNode {
                startup_nodes,
                username,
                password,
                ssl,
                socket_timeout,
            } => cluster_pubsub_output(
                startup_nodes,
                username.as_deref(),
                password.as_deref(),
                *ssl,
                *socket_timeout,
            ),
            Input::ClientKeySame { left, right } => Ok(json!({
                "same": settings_key(left)? == settings_key(right)?,
            })),
        }
    }
}
