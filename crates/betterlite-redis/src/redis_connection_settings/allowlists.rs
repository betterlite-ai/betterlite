mod tests;

use std::collections::BTreeMap;

use super::model::scalars::PassthroughValue;
use super::model::settings::{ClientKind, Target};

// Direct clients accept client and connection settings through Redis.__init__.
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
    "ssl_password",
    "ssl_validate_ocsp",
    "ssl_validate_ocsp_stapled",
    "ssl_ocsp_context",
    "ssl_ocsp_expected_cert",
    "ssl_min_version",
    "ssl_ciphers",
    "legacy_responses",
    "redis_connect_func",
    "gcp_service_account",
    "gcp_ssl_ca_certs",
    "azure_redis_ad_token",
    "azure_client_id",
    "azure_tenant_id",
    "azure_client_secret",
    "aws_iam_auth",
    "aws_iam_user_name",
    "aws_iam_cache_name",
    "aws_iam_region",
    "aws_iam_serverless",
];

// from_url forwards only Connection parameters; rediss selects SSLConnection itself.
const URL_CONNECTION_KWARGS: &[&str] = &[
    "socket_keepalive_options",
    "socket_type",
    "max_connections",
    "health_check_interval",
    "retry_on_timeout",
    "retry_on_error",
    "encoding",
    "encoding_errors",
    "decode_responses",
    "protocol",
    "redis_connect_func",
    "credential_provider",
];

const CLUSTER_KWARGS: &[&str] = &[
    "max_connections",
    "decode_responses",
    "encoding",
    "encoding_errors",
    "retry_on_timeout",
    "retry_on_error",
    "client_name",
    "lib_name",
    "lib_version",
    "protocol",
    "socket_keepalive_options",
    "socket_read_size",
    "socket_type",
    "cluster_error_retry_attempts",
    "require_full_coverage",
    "reinitialize_steps",
    "read_from_replicas",
    "dynamic_startup_nodes",
    "connection_error_retry_attempts",
    "ssl",
    "ssl_cert_reqs",
    "ssl_certfile",
    "ssl_keyfile",
    "ssl_include_verify_flags",
    "ssl_exclude_verify_flags",
    "ssl_check_hostname",
    "ssl_ca_certs",
    "ssl_ca_data",
    "ssl_validate_ocsp",
    "ssl_validate_ocsp_stapled",
    "ssl_ocsp_expected_cert",
    "ssl_min_version",
    "ssl_ciphers",
    "legacy_responses",
    "redis_connect_func",
    "credential_provider",
    "gcp_service_account",
    "gcp_ssl_ca_certs",
    "azure_redis_ad_token",
    "azure_client_id",
    "azure_tenant_id",
    "azure_client_secret",
    "aws_iam_auth",
    "aws_iam_user_name",
    "aws_iam_cache_name",
    "aws_iam_region",
    "aws_iam_serverless",
];

pub fn filter_passthrough_for_target(
    target: Target,
    client: ClientKind,
    passthrough: &mut BTreeMap<String, PassthroughValue>,
) {
    let allowed = match target {
        Target::Direct | Target::Sentinel => DIRECT_KWARGS,
        Target::Url => URL_CONNECTION_KWARGS,
        Target::Cluster => CLUSTER_KWARGS,
    };
    passthrough.retain(|name, _| allowed.contains(&name.as_str()));
    if target == Target::Cluster && client == ClientKind::Sync {
        passthrough.remove("decode_responses");
    }
}
