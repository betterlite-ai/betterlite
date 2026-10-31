mod tests;

use std::collections::BTreeMap;

use super::model::scalars::PassthroughValue;
use super::model::settings::{ClientKind, Target};

pub(super) const DIRECT_COMMON: &[&str] = &[
    "client_name",
    "decode_responses",
    "driver_info",
    "encoding",
    "encoding_errors",
    "event_dispatcher",
    "legacy_responses",
    "lib_name",
    "lib_version",
    "maint_notifications_config",
    "protocol",
    "redis_connect_func",
    "retry_on_error",
    "retry_on_timeout",
    "single_connection_client",
    "socket_keepalive_options",
    "socket_read_size",
    "ssl_ca_certs",
    "ssl_ca_data",
    "ssl_ca_path",
    "ssl_cert_reqs",
    "ssl_certfile",
    "ssl_check_hostname",
    "ssl_ciphers",
    "ssl_exclude_verify_flags",
    "ssl_include_verify_flags",
    "ssl_keyfile",
    "ssl_min_version",
    "unix_socket_path",
];

pub(super) const DIRECT_SYNC_ONLY: &[&str] = &[
    "cache",
    "cache_config",
    "oss_cluster_maint_notifications_handler",
    "ssl_ocsp_context",
    "ssl_ocsp_expected_cert",
    "ssl_validate_ocsp",
    "ssl_validate_ocsp_stapled",
];

pub(super) const DIRECT_ASYNC_ONLY: &[&str] = &["auto_close_connection_pool"];

pub(super) const URL_COMMON: &[&str] = &[
    "max_connections",
    "client_name",
    "decode_responses",
    "driver_info",
    "encoding",
    "encoding_errors",
    "event_dispatcher",
    "himport_registry",
    "legacy_responses",
    "lib_name",
    "lib_version",
    "maint_notifications_config",
    "maint_notifications_pool_handler",
    "maintenance_notification_hash",
    "maintenance_state",
    "orig_host_address",
    "orig_socket_connect_timeout",
    "orig_socket_timeout",
    "oss_cluster_maint_notifications_handler",
    "parser",
    "parser_class",
    "protocol",
    "redis_connect_func",
    "retry_on_error",
    "retry_on_timeout",
    "socket_keepalive_options",
    "socket_read_size",
    "socket_type",
];

pub(super) const URL_SYNC_ONLY: &[&str] = &["command_packer"];

pub(super) const URL_ASYNC_ONLY: &[&str] = &["encoder_class"];

pub(super) const CLUSTER_COMMON: &[&str] = &[
    "max_connections",
    "address_remap",
    "cluster_error_retry_attempts",
    "dynamic_startup_nodes",
    "event_dispatcher",
    "load_balancing_strategy",
    "maint_notifications_config",
    "policy_resolver",
    "read_from_replicas",
    "redis_connect_func",
    "reinitialize_steps",
    "require_full_coverage",
    "ssl_ca_certs",
    "ssl_cert_reqs",
    "ssl_check_hostname",
];

pub(super) const CLUSTER_SYNC_ONLY: &[&str] = &["cache", "cache_config"];

pub(super) const CLUSTER_ASYNC_ONLY: &[&str] = &[
    "client_name",
    "decode_responses",
    "driver_info",
    "encoding",
    "encoding_errors",
    "legacy_responses",
    "lib_name",
    "lib_version",
    "path",
    "protocol",
    "retry_on_error",
    "socket_keepalive_options",
    "socket_read_size",
    "ssl_ca_data",
    "ssl_certfile",
    "ssl_ciphers",
    "ssl_exclude_verify_flags",
    "ssl_include_verify_flags",
    "ssl_keyfile",
    "ssl_min_version",
];

pub fn filter_passthrough_for_target(
    target: Target,
    client: ClientKind,
    passthrough: &mut BTreeMap<String, PassthroughValue>,
) -> Result<(), super::error::RedisSettingsError> {
    let (common, sync, asynchronous) = match target {
        Target::Direct | Target::Sentinel => (DIRECT_COMMON, DIRECT_SYNC_ONLY, DIRECT_ASYNC_ONLY),
        Target::Url => (URL_COMMON, URL_SYNC_ONLY, URL_ASYNC_ONLY),
        Target::Cluster => (CLUSTER_COMMON, CLUSTER_SYNC_ONLY, CLUSTER_ASYNC_ONLY),
    };
    let specific = match client {
        ClientKind::Sync => sync,
        ClientKind::Async => asynchronous,
    };
    let allowed = |name: &str| common.contains(&name) || specific.contains(&name);
    if target == Target::Direct {
        if let Some(name) = passthrough.keys().find(|name| !allowed(name)) {
            return Err(super::error::RedisSettingsError::UnknownOption { name: name.clone() });
        }
    } else {
        passthrough.retain(|name, _| allowed(name));
    }
    Ok(())
}
