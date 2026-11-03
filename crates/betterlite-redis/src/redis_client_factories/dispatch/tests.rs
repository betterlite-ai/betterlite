#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret;
use std::collections::BTreeMap;

use super::build_client;
use crate::redis_client_factories::RedisClient;
use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::{
    ClientKind, CredentialProviderSettings, RedisConnectionSettings, Target,
};

fn blank_settings() -> RedisConnectionSettings {
    RedisConnectionSettings {
        host: None,
        port: None,
        db: None,
        username: None,
        password: None,
        url: None,
        ssl: None,
        socket_timeout: None,
        socket_connect_timeout: None,
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
        credential_provider: false,
        credential_provider_settings: None,
        startup_nodes: None,
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        ssl_password: None,
        passthrough: BTreeMap::new(),
    }
}

#[test]
fn startup_nodes_dispatch_to_a_cluster_client() {
    let settings = RedisConnectionSettings {
        startup_nodes: Some(vec![ClusterNodeConfig {
            host: "node-a".to_owned(),
            port: 7000,
        }]),
        ..blank_settings()
    };
    assert!(matches!(
        build_client(&settings, ClientKind::Sync).expect("startup_nodes resolves"),
        RedisClient::Cluster(_)
    ));
    assert!(matches!(
        build_client(&settings, ClientKind::Async).expect("startup_nodes resolves"),
        RedisClient::Cluster(_)
    ));
}

#[test]
fn a_url_dispatches_to_a_url_client() {
    let settings = RedisConnectionSettings {
        url: Some(secrecy::SecretString::from("redis://cache:6379".to_owned())),
        ..blank_settings()
    };
    assert!(matches!(
        build_client(&settings, ClientKind::Sync).expect("url resolves"),
        RedisClient::Url(_)
    ));
}

#[test]
fn sentinel_nodes_and_service_name_dispatch_to_a_sentinel_client() {
    let settings = RedisConnectionSettings {
        sentinel_nodes: Some(vec![("sentinel-1".to_owned(), 26379)]),
        service_name: Some("mymaster".to_owned()),
        ..blank_settings()
    };
    assert!(matches!(
        build_client(&settings, ClientKind::Sync).expect("sentinel_nodes and service_name resolve"),
        RedisClient::Sentinel(_)
    ));
}

#[test]
fn a_bare_host_dispatches_to_a_direct_client() {
    let settings = RedisConnectionSettings {
        host: Some("cache".to_owned()),
        ..blank_settings()
    };
    assert!(matches!(
        build_client(&settings, ClientKind::Sync).expect("host resolves"),
        RedisClient::Direct(_)
    ));
}

#[test]
fn startup_nodes_take_priority_over_every_other_target() {
    let settings = RedisConnectionSettings {
        startup_nodes: Some(vec![ClusterNodeConfig {
            host: "node-a".to_owned(),
            port: 7000,
        }]),
        url: Some(secrecy::SecretString::from("redis://cache:6379".to_owned())),
        sentinel_nodes: Some(vec![("sentinel-1".to_owned(), 26379)]),
        service_name: Some("mymaster".to_owned()),
        host: Some("cache".to_owned()),
        ..blank_settings()
    };
    assert!(matches!(
        build_client(&settings, ClientKind::Sync).expect("startup_nodes resolves"),
        RedisClient::Cluster(_)
    ));
}

#[test]
fn an_async_cluster_dispatch_carries_isolation_but_a_sync_one_does_not() {
    let settings = RedisConnectionSettings {
        startup_nodes: Some(vec![ClusterNodeConfig {
            host: "node-a".to_owned(),
            port: 7000,
        }]),
        ..blank_settings()
    };
    let RedisClient::Cluster(sync_client) =
        build_client(&settings, ClientKind::Sync).expect("resolves")
    else {
        panic!("expected a cluster client");
    };
    let RedisClient::Cluster(async_client) =
        build_client(&settings, ClientKind::Async).expect("resolves")
    else {
        panic!("expected a cluster client");
    };
    assert!(sync_client.isolation.is_none());
    assert!(async_client.isolation.is_some());
}

#[test]
fn no_target_fields_set_is_an_error() {
    build_client(&blank_settings(), ClientKind::Sync).expect_err("no target field is set");
}

#[rstest]
#[case::sync_cluster_gcp(Target::Cluster, ClientKind::Sync, false)]
#[case::async_cluster_azure(Target::Cluster, ClientKind::Async, true)]
#[case::async_url_gcp(Target::Url, ClientKind::Async, false)]
#[case::async_sentinel_azure(Target::Sentinel, ClientKind::Async, true)]
#[case::async_direct_gcp(Target::Direct, ClientKind::Async, false)]
fn iam_provider_dispatch_replaces_static_credentials(
    #[case] target: Target,
    #[case] kind: ClientKind,
    #[case] azure: bool,
) {
    let startup_nodes = (target == Target::Cluster).then(|| {
        vec![ClusterNodeConfig {
            host: "node-a".to_owned(),
            port: 7000,
        }]
    });
    let url = (target == Target::Url).then_some("redis://cache:6379");
    let sentinel_nodes = (target == Target::Sentinel).then(|| vec![("sentinel".to_owned(), 26379)]);
    let service_name = (target == Target::Sentinel).then(|| "master".to_owned());
    let host = (target == Target::Direct).then(|| "cache".to_owned());
    let provider = if azure {
        CredentialProviderSettings::Azure {
            client_id: None,
            tenant_id: None,
            client_secret: None,
        }
    } else {
        CredentialProviderSettings::Gcp {
            service_account: "svc".to_owned(),
            ssl_ca_certs: None,
        }
    };
    let settings = RedisConnectionSettings {
        startup_nodes,
        sentinel_nodes,
        service_name,
        host,
        url: url.map(|value| secrecy::SecretString::from(value.to_owned())),
        username: Some("user".to_owned()),
        password: Some(secrecy::SecretString::from("pass".to_owned())),
        sentinel_password: Some(secrecy::SecretString::from("monitor".to_owned())),
        credential_provider_settings: Some(provider),
        passthrough: BTreeMap::from([(
            "redis_connect_func".to_owned(),
            PassthroughValue::Text("callback".to_owned()),
        )]),
        ..blank_settings()
    };
    let built_client = build_client(&settings, kind).expect("client builds");
    let should_replace = kind == ClientKind::Async || target == Target::Cluster;
    match built_client {
        RedisClient::Cluster(cluster_client) => {
            assert_eq!(cluster_client.credential_provider, should_replace);
            assert_eq!(cluster_client.username.is_none(), should_replace);
            assert_eq!(cluster_client.password.is_none(), should_replace);
            assert_eq!(
                cluster_client
                    .passthrough
                    .contains_key("redis_connect_func"),
                !should_replace
            );
        }
        RedisClient::Url(url_client) => {
            assert_eq!(url_client.credential_provider, should_replace);
            assert_eq!(url_client.username.is_none(), should_replace);
            assert_eq!(
                url_client.passthrough.contains_key("redis_connect_func"),
                !should_replace
            );
        }
        RedisClient::Sentinel(sentinel_client) => {
            assert_eq!(
                sentinel_client.connection_kwargs.credential_provider,
                should_replace
            );
            assert_eq!(
                sentinel_client.connection_kwargs.username.is_none(),
                should_replace
            );
            assert_eq!(
                sentinel_client.connection_kwargs.password.is_none(),
                should_replace
            );
            assert_eq!(
                sentinel_client
                    .monitor_password
                    .as_ref()
                    .map(ExposeSecret::expose_secret),
                Some("monitor")
            );
        }
        RedisClient::Direct(direct_client) => {
            assert_eq!(direct_client.credential_provider, should_replace);
            assert_eq!(direct_client.username.is_none(), should_replace);
            assert_eq!(direct_client.password.is_none(), should_replace);
        }
    }
}

#[rstest]
#[case::sync_url_gcp(Target::Url, false)]
#[case::sync_sentinel_azure(Target::Sentinel, true)]
#[case::sync_direct_gcp(Target::Direct, false)]
fn iam_provider_dispatch_keeps_static_credentials_on_sync_paths(
    #[case] target: Target,
    #[case] azure: bool,
) {
    let target_settings = match target {
        Target::Url => RedisConnectionSettings {
            url: Some(secrecy::SecretString::from("redis://cache:6379".to_owned())),
            ..blank_settings()
        },
        Target::Sentinel => RedisConnectionSettings {
            sentinel_nodes: Some(vec![("sentinel".to_owned(), 26379)]),
            service_name: Some("master".to_owned()),
            ..blank_settings()
        },
        Target::Direct => RedisConnectionSettings {
            host: Some("cache".to_owned()),
            ..blank_settings()
        },
        Target::Cluster => blank_settings(),
    };
    let provider_settings = if azure {
        CredentialProviderSettings::Azure {
            client_id: None,
            tenant_id: None,
            client_secret: None,
        }
    } else {
        CredentialProviderSettings::Gcp {
            service_account: "svc".to_owned(),
            ssl_ca_certs: None,
        }
    };
    let settings = RedisConnectionSettings {
        username: Some("user".to_owned()),
        password: Some(secrecy::SecretString::from("pass".to_owned())),
        credential_provider_settings: Some(provider_settings),
        passthrough: BTreeMap::from([(
            "redis_connect_func".to_owned(),
            PassthroughValue::Text("callback".to_owned()),
        )]),
        ..target_settings
    };
    let built_client = build_client(&settings, ClientKind::Sync).expect("sync client builds");
    assert!(match built_client {
        RedisClient::Url(url_client) =>
            url_client.username.is_some()
                && !url_client.credential_provider
                && url_client.passthrough.contains_key("redis_connect_func"),
        RedisClient::Sentinel(sentinel_client) =>
            sentinel_client.connection_kwargs.username.is_some()
                && sentinel_client
                    .connection_kwargs
                    .password
                    .as_ref()
                    .is_some_and(|password| password.expose_secret() == "pass")
                && !sentinel_client.connection_kwargs.credential_provider
                && sentinel_client
                    .connection_kwargs
                    .passthrough
                    .contains_key("redis_connect_func"),
        RedisClient::Direct(direct_client) =>
            direct_client.username.is_some()
                && direct_client
                    .password
                    .as_ref()
                    .is_some_and(|password| password.expose_secret() == "pass")
                && !direct_client.credential_provider
                && direct_client.passthrough.contains_key("redis_connect_func"),
        RedisClient::Cluster(_) => false,
    });
}
