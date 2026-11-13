#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};

fn same(left: f64, right: f64) -> bool {
    left.to_bits() == right.to_bits()
}

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .routing_state_store)
}

fn configured(source: &str) -> super::Settings {
    load(source, &[]).unwrap()
}

#[rstest]
#[case::defaults_are_unset("defaults_are_unset")]
#[case::explicit_values("explicit_values")]
#[case::environment_fallbacks("environment_fallbacks")]
#[case::explicit_url_wins_over_the_environment("explicit_url_wins_over_the_environment")]
#[case::socket_timeout_precedence("socket_timeout_precedence")]
#[case::null_socket_timeout_uses_the_shared_client_default(
    "null_socket_timeout_uses_the_shared_client_default"
)]
#[case::cache_kwargs_outrank_redis_kwargs("cache_kwargs_outrank_redis_kwargs")]
#[case::redis_environment_overlay("redis_environment_overlay")]
#[case::os_environ_references("os_environ_references")]
#[case::missing_environment_names_the_variable("missing_environment_names_the_variable")]
#[case::invalid_port_types("invalid_port_types")]
#[case::unknown_nested_key_is_reported("unknown_nested_key_is_reported")]
#[case::empty_namespace_is_kept("empty_namespace_is_kept")]
#[case::empty_environment_values_are_unset("empty_environment_values_are_unset")]
#[case::connection_options_are_read("connection_options_are_read")]
#[case::connection_option_precedence("connection_option_precedence")]
#[case::connection_options_default_to_unset("connection_options_default_to_unset")]
#[case::connection_option_types_are_checked("connection_option_types_are_checked")]
#[case::iam_environment_names_reach_the_overlay("iam_environment_names_reach_the_overlay")]
fn routing_state_store_config(#[case] name: &str) {
    match name {
        "defaults_are_unset" => {
            let settings = configured("");
            assert!(settings.redis_url.is_none());
            assert!(settings.redis_host.is_none());
            assert!(settings.redis_port.is_none());
            assert!(settings.redis_password.is_none());
            assert!(settings.namespace.is_none());
            assert!(same(settings.socket_timeout, 5.0_f64));
            assert_eq!(settings.pool_timeout_seconds, 5);
            assert!(settings.redis_environment.is_empty());
        }
        "explicit_values" => {
            let settings = configured(
                "router_settings:\n  redis_url: redis://cache.internal:6380/2\n  redis_host: cache.internal\n  redis_port: 6380\n  redis_password: hunter2\n  cache_kwargs:\n    namespace: router\n    socket_timeout: 1.5\n",
            );
            assert_eq!(
                settings.redis_url.unwrap().expose_secret(),
                "redis://cache.internal:6380/2"
            );
            assert_eq!(settings.redis_host.as_deref(), Some("cache.internal"));
            assert_eq!(settings.redis_port, Some(6380));
            assert_eq!(settings.redis_password.unwrap().expose_secret(), "hunter2");
            assert_eq!(settings.namespace.as_deref(), Some("router"));
            assert!(same(settings.socket_timeout, 1.5_f64));
        }
        "environment_fallbacks" => {
            let settings = load(
                "",
                &[
                    ("REDIS_URL", "redis://fallback:6379"),
                    ("REDIS_HOST", "fallback"),
                    ("REDIS_PORT", "6390"),
                    ("REDIS_PASSWORD", "from-env"),
                    ("REDIS_SOCKET_TIMEOUT", "0.1"),
                    ("REDIS_CONNECTION_POOL_TIMEOUT", "9"),
                ],
            )
            .unwrap();
            assert!(settings.redis_url.is_none());
            assert!(settings.redis_host.is_none());
            assert!(settings.redis_port.is_none());
            assert!(settings.redis_password.is_none());
            assert!(same(settings.socket_timeout, 5.0_f64));
            assert_eq!(settings.pool_timeout_seconds, 9);
        }
        "explicit_url_wins_over_the_environment" => {
            let settings = load(
                "router_settings:\n  redis_url: redis://explicit:6379\n",
                &[("REDIS_URL", "redis://fallback:6379")],
            )
            .unwrap();
            assert_eq!(
                settings.redis_url.unwrap().expose_secret(),
                "redis://explicit:6379"
            );
        }
        "socket_timeout_precedence" => {
            let timeout = |source: &str| load(source, &[("REDIS_SOCKET_TIMEOUT", "0.7")]).unwrap();
            assert!(same(
                timeout("router_settings:\n  cache_kwargs:\n    socket_timeout: 2.5\n")
                    .socket_timeout,
                2.5_f64
            ));
            assert!(same(
                timeout("router_settings:\n  redis_kwargs:\n    socket_timeout: 3\n")
                    .socket_timeout,
                3.0_f64
            ));
            assert!(same(timeout("").socket_timeout, 5.0_f64));
            assert!(same(configured("").socket_timeout, 5.0_f64));
        }
        "null_socket_timeout_uses_the_shared_client_default" => {
            let source = "router_settings:\n  cache_kwargs:\n    socket_timeout: null\n";
            assert!(same(
                load(source, &[("REDIS_SOCKET_TIMEOUT", "0.7")])
                    .unwrap()
                    .socket_timeout,
                0.7_f64
            ));
            assert!(same(configured(source).socket_timeout, 0.1_f64));
            assert!(same(
                load(source, &[("REDIS_SOCKET_TIMEOUT", "")])
                    .unwrap()
                    .socket_timeout,
                0.1_f64
            ));
        }
        "cache_kwargs_outrank_redis_kwargs" => {
            let settings = configured(
                "router_settings:\n  cache_kwargs:\n    namespace: cache\n    socket_timeout: 1\n  redis_kwargs:\n    namespace: redis\n    socket_timeout: 2\n",
            );
            assert_eq!(settings.namespace.as_deref(), Some("cache"));
            assert!(same(settings.socket_timeout, 1.0_f64));
            let only_redis =
                configured("router_settings:\n  redis_kwargs:\n    namespace: redis\n");
            assert_eq!(only_redis.namespace.as_deref(), Some("redis"));
        }
        "redis_environment_overlay" => {
            let settings = load(
                "",
                &[
                    ("REDIS_USERNAME", "router"),
                    ("REDIS_SSL", "true"),
                    ("REDIS_DB", "3"),
                    ("REDIS_ENCODING", ""),
                    ("REDIS_NOT_A_KWARG", "x"),
                ],
            )
            .unwrap();
            let overlay: BTreeMap<_, _> = settings
                .redis_environment
                .iter()
                .map(|(variable, value)| (variable.as_str(), value.expose_secret()))
                .collect();
            assert_eq!(
                overlay,
                BTreeMap::from([
                    ("REDIS_DB", "3"),
                    ("REDIS_SSL", "true"),
                    ("REDIS_USERNAME", "router")
                ])
            );
        }
        "os_environ_references" => {
            let settings = load(
                "router_settings:\n  redis_url: os.environ/CACHE_URL\n  redis_password: os.environ/CACHE_PASSWORD\n  cache_kwargs:\n    namespace: os.environ/CACHE_NAMESPACE\n",
                &[
                    ("CACHE_URL", "redis://secret-host:6379"),
                    ("CACHE_PASSWORD", "secret"),
                    ("CACHE_NAMESPACE", "from-env"),
                ],
            )
            .unwrap();
            assert_eq!(
                settings.redis_url.unwrap().expose_secret(),
                "redis://secret-host:6379"
            );
            assert_eq!(settings.redis_password.unwrap().expose_secret(), "secret");
            assert_eq!(settings.namespace.as_deref(), Some("from-env"));
        }
        "missing_environment_names_the_variable" => {
            let error =
                load("router_settings:\n  redis_url: os.environ/CACHE_URL\n", &[]).unwrap_err();
            assert!(matches!(
                error,
                ConfigError::MissingEnvironment { variable, .. } if variable == "CACHE_URL"
            ));
        }
        "invalid_port_types" => {
            assert!(matches!(
                load("router_settings:\n  redis_port: true\n", &[]),
                Err(ConfigError::InvalidScalar { .. })
            ));
            assert!(matches!(
                load("router_settings:\n  redis_port: many\n", &[]),
                Err(ConfigError::InvalidInteger { .. })
            ));
            let settings = load("", &[("REDIS_PORT", "many")]).unwrap();
            assert_eq!(settings.redis_port, None);
            assert_eq!(
                settings.redis_environment["REDIS_PORT"].expose_secret(),
                "many"
            );
        }
        "unknown_nested_key_is_reported" => {
            let error = load(
                "router_settings:\n  redis_kwargs:\n    startup_nodes: []\n  cache_kwargs:\n    startup_nodes: []\n",
                &[],
            )
            .unwrap_err();
            let ConfigError::Unsupported { paths } = error else {
                panic!("expected an unsupported-key error");
            };
            for key in ["redis_kwargs", "cache_kwargs"] {
                let path = format!("router_settings.{key}.startup_nodes");
                assert!(paths.contains(&path), "{path} missing from {paths:?}");
            }
        }
        "empty_namespace_is_kept" => {
            let settings = configured("router_settings:\n  cache_kwargs:\n    namespace: ''\n");
            assert_eq!(settings.namespace.as_deref(), Some(""));
        }
        "empty_environment_values_are_unset" => {
            let settings = load(
                "",
                &[
                    ("REDIS_SOCKET_TIMEOUT", ""),
                    ("REDIS_CONNECTION_POOL_TIMEOUT", ""),
                    ("REDIS_PORT", ""),
                ],
            )
            .unwrap();
            assert!(same(settings.socket_timeout, 5.0_f64));
            assert_eq!(settings.pool_timeout_seconds, 5);
            assert!(settings.redis_port.is_none());
        }
        "connection_options_are_read" => {
            let settings = configured(
                "router_settings:\n  cache_kwargs:\n    db: 2\n    username: cache-user\n    ssl: true\n    socket_connect_timeout: 2.5\n    socket_keepalive: false\n    max_connections: 10\n    health_check_interval: 30\n",
            );
            let connection = settings.connection;
            assert_eq!(connection.db, Some(2));
            assert_eq!(connection.username.as_deref(), Some("cache-user"));
            assert_eq!(connection.ssl, Some(true));
            assert!(
                connection
                    .socket_connect_timeout
                    .is_some_and(|seconds| same(seconds, 2.5_f64))
            );
            assert_eq!(connection.socket_keepalive, Some(false));
            assert_eq!(connection.max_connections, Some(10));
            assert_eq!(connection.health_check_interval, Some(30));
        }
        "connection_option_precedence" => {
            let db = |source: &str| configured(source).connection.db;
            assert_eq!(db("router_settings:\n  redis_db: 1\n"), Some(1));
            assert_eq!(
                db("router_settings:\n  redis_db: 1\n  cache_kwargs:\n    db: 2\n"),
                Some(2)
            );
            assert_eq!(
                db("router_settings:\n  redis_db: 1\n  redis_kwargs:\n    db: 5\n"),
                Some(1)
            );
            assert_eq!(
                db("router_settings:\n  redis_kwargs:\n    db: 5\n"),
                Some(5)
            );
            let settings = configured(
                "router_settings:\n  cache_kwargs:\n    ssl: true\n  redis_kwargs:\n    ssl: false\n    username: fallback\n    max_connections: 7\n",
            );
            assert_eq!(settings.connection.ssl, Some(true));
            assert_eq!(settings.connection.username.as_deref(), Some("fallback"));
            assert_eq!(settings.connection.max_connections, Some(7));
            let referenced = load(
                "router_settings:\n  redis_db: os.environ/CACHE_DB\n  cache_kwargs:\n    username: os.environ/CACHE_USER\n",
                &[("CACHE_DB", "6"), ("CACHE_USER", "from-env")],
            )
            .unwrap();
            assert_eq!(referenced.connection.db, Some(6));
            assert_eq!(referenced.connection.username.as_deref(), Some("from-env"));
        }
        "connection_options_default_to_unset" => {
            let connection = load("", &[("REDIS_DB", "3"), ("REDIS_SSL", "true")])
                .unwrap()
                .connection;
            assert!(connection.db.is_none());
            assert!(connection.username.is_none());
            assert!(connection.ssl.is_none());
            assert!(connection.socket_connect_timeout.is_none());
            assert!(connection.socket_keepalive.is_none());
            assert!(connection.max_connections.is_none());
            assert!(connection.health_check_interval.is_none());
        }
        "connection_option_types_are_checked" => {
            assert!(matches!(
                load("router_settings:\n  redis_db: first\n", &[]),
                Err(ConfigError::InvalidInteger { .. })
            ));
            assert!(matches!(
                load(
                    "router_settings:\n  cache_kwargs:\n    max_connections: many\n",
                    &[]
                ),
                Err(ConfigError::InvalidInteger { .. })
            ));
            assert!(matches!(
                load(
                    "router_settings:\n  cache_kwargs:\n    ssl: sometimes\n",
                    &[]
                ),
                Err(ConfigError::Yaml { .. })
            ));
            assert!(matches!(
                load("router_settings:\n  cache_kwargs:\n    db: os.environ/MISSING_DB\n", &[]),
                Err(ConfigError::MissingEnvironment { variable, .. }) if variable == "MISSING_DB"
            ));
        }
        "iam_environment_names_reach_the_overlay" => {
            let names = [
                "REDIS_GCP_SERVICE_ACCOUNT",
                "REDIS_GCP_SSL_CA_CERTS",
                "REDIS_AZURE_REDIS_AD_TOKEN",
                "REDIS_AZURE_AD_TOKEN",
                "REDIS_AZURE_CLIENT_ID",
                "REDIS_AZURE_TENANT_ID",
                "REDIS_AZURE_CLIENT_SECRET",
                "AZURE_CLIENT_ID",
                "AZURE_TENANT_ID",
                "AZURE_CLIENT_SECRET",
                "REDIS_AWS_IAM_AUTH",
                "REDIS_AWS_IAM_USER_NAME",
                "REDIS_AWS_IAM_CACHE_NAME",
                "REDIS_AWS_IAM_REGION",
                "REDIS_AWS_IAM_SERVERLESS",
                "AWS_REGION",
                "AWS_DEFAULT_REGION",
            ];
            let flags = ["REDIS_AWS_IAM_AUTH", "REDIS_AWS_IAM_SERVERLESS"];
            let environment: Vec<(&str, &str)> = names
                .iter()
                .map(|variable| {
                    (
                        *variable,
                        if flags.contains(variable) {
                            "true"
                        } else {
                            "set"
                        },
                    )
                })
                .collect();
            let settings = load("", &environment).unwrap();
            for variable in names {
                assert!(
                    settings.redis_environment.contains_key(variable),
                    "{variable} missing"
                );
            }
            assert_eq!(settings.redis_environment.len(), names.len());
        }
        other => panic!("unknown case {other}"),
    }
}
