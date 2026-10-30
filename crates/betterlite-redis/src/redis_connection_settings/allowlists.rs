mod tests;

use std::collections::BTreeMap;

use super::model::scalars::PassthroughValue;
use super::model::settings::{ClientKind, Target};

// litellm._redis._get_redis_cluster_kwargs excludes these; a cluster reaches them through startup_nodes instead.
const NON_CLUSTER_ONLY_KWARGS: &[&str] = &["host", "port", "startup_nodes"];

// The same function adds these for every cluster client; redis.Redis and url connections never accept them.
const CLUSTER_ONLY_KWARGS: &[&str] = &[
    "cluster_error_retry_attempts",
    "require_full_coverage",
    "reinitialize_steps",
    "read_from_replicas",
    "dynamic_startup_nodes",
    "connection_error_retry_attempts",
    "ssl_cert_reqs",
    "ssl_check_hostname",
    "ssl_ca_certs",
];

// redis.asyncio.cluster.RedisCluster names decode_responses; the sync client only takes it through **kwargs.
const CLUSTER_ASYNC_ONLY_KWARGS: &[&str] = &["decode_responses"];

pub fn filter_passthrough_for_target(
    target: Target,
    client: ClientKind,
    passthrough: &mut BTreeMap<String, PassthroughValue>,
) {
    match target {
        Target::Cluster => filter_for_cluster(client, passthrough),
        Target::Direct | Target::Url | Target::Sentinel => filter_for_non_cluster(passthrough),
    }
}

fn filter_for_cluster(client: ClientKind, passthrough: &mut BTreeMap<String, PassthroughValue>) {
    for name in NON_CLUSTER_ONLY_KWARGS {
        passthrough.remove(*name);
    }
    if client == ClientKind::Sync {
        for name in CLUSTER_ASYNC_ONLY_KWARGS {
            passthrough.remove(*name);
        }
    }
}

fn filter_for_non_cluster(passthrough: &mut BTreeMap<String, PassthroughValue>) {
    for name in CLUSTER_ONLY_KWARGS {
        passthrough.remove(*name);
    }
    for name in CLUSTER_ASYNC_ONLY_KWARGS {
        passthrough.remove(*name);
    }
}
