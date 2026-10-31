mod tests;

use std::collections::BTreeMap;

use super::model::scalars::PassthroughValue;
use super::model::settings::{ClientKind, Target};

// Direct clients accept client and connection settings through Redis.__init__. gcp_*, azure_*, aws_iam_* and ssl_password are no longer listed: RedisOverrides now models them as dedicated fields (see resolve::iam and model::settings), so they never reach `passthrough`.
const DIRECT_KWARGS: &[&str] = &[
    "encoding",
    "encoding_errors",
    "socket_read_size",
    "unix_socket_path",
    "decode_responses",
    "retry_on_timeout",
    "retry_on_error",
    "client_name",
    "lib_name",
    "lib_version",
    "protocol",
    "socket_keepalive_options",
    "single_connection_client",
    "auto_close_connection_pool",
    "ssl",
    "ssl_cert_reqs",
    "ssl_keyfile",
    "ssl_certfile",
    "ssl_include_verify_flags",
    "ssl_exclude_verify_flags",
    "ssl_check_hostname",
    "ssl_ca_certs",
    "ssl_ca_path",
    "ssl_ca_data",
    "ssl_validate_ocsp",
    "ssl_validate_ocsp_stapled",
    "ssl_ocsp_context",
    "ssl_ocsp_expected_cert",
    "ssl_min_version",
    "ssl_ciphers",
    "legacy_responses",
    "redis_connect_func",
];

// from_url forwards only Connection parameters; rediss selects SSLConnection itself.
const URL_CONNECTION_KWARGS: &[&str] = &[
    "socket_keepalive_options",
    "socket_type",
    "socket_read_size",
    "max_connections",
    "health_check_interval",
    "retry_on_timeout",
    "retry_on_error",
    "encoding",
    "encoding_errors",
    "decode_responses",
    "protocol",
    "client_name",
    "lib_name",
    "lib_version",
    "legacy_responses",
    "redis_connect_func",
    "credential_provider",
];

// redis.RedisCluster.__init__ only declares cluster-topology and a handful of client parameters itself; everything else flows to each node's Redis() through **kwargs, so litellm's _get_redis_cluster_kwargs re-adds that fixed set by hand (see litellm/_redis.py#L150-L189). This mirrors that exact union for the sync constructor.
const CLUSTER_SYNC_KWARGS: &[&str] = &[
    "cluster_error_retry_attempts",
    "require_full_coverage",
    "reinitialize_steps",
    "read_from_replicas",
    "dynamic_startup_nodes",
    "password",
    "username",
    "ssl",
    "ssl_cert_reqs",
    "ssl_check_hostname",
    "ssl_ca_certs",
    "redis_connect_func",
    "credential_provider",
    "max_connections",
    "socket_timeout",
    "socket_connect_timeout",
    "health_check_interval",
    "socket_keepalive",
];

// async_redis.RedisCluster.__init__ declares client and connection parameters directly (no **kwargs passthrough to per-node clients), so its allowlist is the constructor's own signature unioned with the same fixed set litellm adds for every Cluster client.
const CLUSTER_ASYNC_KWARGS: &[&str] = &[
    "require_full_coverage",
    "read_from_replicas",
    "dynamic_startup_nodes",
    "reinitialize_steps",
    "cluster_error_retry_attempts",
    "max_connections",
    "retry_on_error",
    "credential_provider",
    "username",
    "password",
    "client_name",
    "lib_name",
    "lib_version",
    "encoding",
    "encoding_errors",
    "decode_responses",
    "health_check_interval",
    "socket_timeout",
    "socket_connect_timeout",
    "socket_read_size",
    "socket_keepalive",
    "socket_keepalive_options",
    "ssl",
    "ssl_ca_certs",
    "ssl_ca_data",
    "ssl_cert_reqs",
    "ssl_include_verify_flags",
    "ssl_exclude_verify_flags",
    "ssl_certfile",
    "ssl_check_hostname",
    "ssl_keyfile",
    "ssl_min_version",
    "ssl_ciphers",
    "protocol",
    "legacy_responses",
    "redis_connect_func",
];

// Url, Cluster and Sentinel targets build their kwargs by keeping only names litellm's own allowlist for that constructor recognizes, silently dropping the rest (litellm/_redis.py's `if arg in args: kept[arg] = value` loops never raise). A bare Direct connection instead forwards redis_kwargs unfiltered straight into Redis.__init__, so an option that signature does not accept is a real error there, not a silent drop.
pub fn filter_passthrough_for_target(
    target: Target,
    client: ClientKind,
    passthrough: &mut BTreeMap<String, PassthroughValue>,
) -> Result<(), super::error::RedisSettingsError> {
    match target {
        Target::Direct => {
            if let Some(name) = passthrough
                .keys()
                .find(|name| !DIRECT_KWARGS.contains(&name.as_str()))
            {
                return Err(super::error::RedisSettingsError::UnknownOption { name: name.clone() });
            }
            Ok(())
        }
        Target::Sentinel => {
            passthrough.retain(|name, _| DIRECT_KWARGS.contains(&name.as_str()));
            Ok(())
        }
        Target::Url => {
            passthrough.retain(|name, _| URL_CONNECTION_KWARGS.contains(&name.as_str()));
            Ok(())
        }
        Target::Cluster => {
            let allowed = if client == ClientKind::Sync {
                CLUSTER_SYNC_KWARGS
            } else {
                CLUSTER_ASYNC_KWARGS
            };
            passthrough.retain(|name, _| allowed.contains(&name.as_str()));
            Ok(())
        }
    }
}
