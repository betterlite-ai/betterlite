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
#[case::explicit_host_suppresses_redis_url("explicit_host_suppresses_redis_url")]
#[case::explicit_url_wins_over_the_environment("explicit_url_wins_over_the_environment")]
#[case::socket_timeout_precedence("socket_timeout_precedence")]
#[case::os_environ_references("os_environ_references")]
#[case::missing_environment_names_the_variable("missing_environment_names_the_variable")]
#[case::invalid_port_types("invalid_port_types")]
#[case::unknown_nested_key_is_reported("unknown_nested_key_is_reported")]
#[case::empty_namespace_is_kept("empty_namespace_is_kept")]
#[case::empty_environment_values_are_unset("empty_environment_values_are_unset")]
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
        }
        "explicit_values" => {
            let settings = configured(
                "router_settings:\n  redis_url: redis://cache.internal:6380/2\n  redis_host: cache.internal\n  redis_port: 6380\n  redis_password: hunter2\n  redis_kwargs:\n    namespace: router\n    socket_timeout: 1.5\n",
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
            assert_eq!(
                settings.redis_url.unwrap().expose_secret(),
                "redis://fallback:6379"
            );
            assert_eq!(settings.redis_host.as_deref(), Some("fallback"));
            assert_eq!(settings.redis_port, Some(6390));
            assert_eq!(settings.redis_password.unwrap().expose_secret(), "from-env");
            assert!(same(settings.socket_timeout, 0.1_f64));
            assert_eq!(settings.pool_timeout_seconds, 9);
        }
        "explicit_host_suppresses_redis_url" => {
            let settings = load(
                "router_settings:\n  redis_host: explicit\n",
                &[("REDIS_URL", "redis://fallback:6379")],
            )
            .unwrap();
            assert!(settings.redis_url.is_none());
            assert_eq!(settings.redis_host.as_deref(), Some("explicit"));
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
            assert!(same(
                load(
                    "router_settings:\n  redis_kwargs:\n    socket_timeout: 2.5\n",
                    &[("REDIS_SOCKET_TIMEOUT", "0.1")]
                )
                .unwrap()
                .socket_timeout,
                2.5_f64
            ));
            assert!(same(
                load("", &[("REDIS_SOCKET_TIMEOUT", "0.1")])
                    .unwrap()
                    .socket_timeout,
                0.1_f64
            ));
            assert!(same(configured("").socket_timeout, 5.0_f64));
        }
        "os_environ_references" => {
            let settings = load(
                "router_settings:\n  redis_url: os.environ/CACHE_URL\n  redis_password: os.environ/CACHE_PASSWORD\n  redis_kwargs:\n    namespace: os.environ/CACHE_NAMESPACE\n",
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
            assert!(matches!(
                load("", &[("REDIS_PORT", "many")]),
                Err(ConfigError::Environment { .. })
            ));
        }
        "unknown_nested_key_is_reported" => {
            let error = load(
                "router_settings:\n  redis_kwargs:\n    startup_nodes: []\n",
                &[],
            )
            .unwrap_err();
            assert!(matches!(
                error,
                ConfigError::Unsupported { paths }
                    if paths.iter().any(|path| path == "router_settings.redis_kwargs.startup_nodes")
            ));
        }
        "empty_namespace_is_kept" => {
            let settings = configured("router_settings:\n  redis_kwargs:\n    namespace: ''\n");
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
        other => panic!("unknown case {other}"),
    }
}
