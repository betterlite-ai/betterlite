use std::str;

use redis::cluster_async::ClusterConnection;
use redis::cluster_routing::{RoutingInfo, SingleNodeRoutingInfo};
use redis::{Value, cmd};

use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::redis_live_client::{LiveAsyncClient, LiveClientError};

use super::reply::{RedisReply, classify};
use super::{RedisCounters, RedisCountersError};

pub(crate) const SCAN_OPERATION: &str = "async_scan_iter";

pub(crate) async fn scan(
    counters: &RedisCounters,
    pattern: &str,
    count: usize,
) -> Result<Vec<Vec<u8>>, RedisCountersError> {
    let namespaced = check_and_fix_namespace(counters.namespace.as_deref(), Some(pattern))
        .unwrap_or_else(|| pattern.to_owned());
    let match_pattern = format!("{namespaced}*");
    let client = counters.transport.async_client().await?;
    match &client {
        LiveAsyncClient::Cluster(cluster) => {
            scan_cluster(&client, cluster, &match_pattern, count).await
        }
        LiveAsyncClient::Standalone(_) | LiveAsyncClient::Sentinel(_) => {
            scan_connection(&client, &match_pattern, count).await
        }
    }
}

async fn scan_cluster(
    client: &LiveAsyncClient,
    cluster: &ClusterConnection,
    match_pattern: &str,
    count: usize,
) -> Result<Vec<Vec<u8>>, RedisCountersError> {
    let mut topology_command = cmd("CLUSTER");
    topology_command.arg("SLOTS");
    let topology: Value = client.query(&topology_command).await?;
    let mut keys = Vec::new();
    for (host, port) in cluster_primaries(topology)? {
        if keys.len() >= count {
            break;
        }
        let route = ScanRoute::Node {
            cluster,
            host,
            port,
        };
        collect_scan(&route, match_pattern, count, &mut keys).await?;
    }
    Ok(keys)
}

async fn scan_connection(
    client: &LiveAsyncClient,
    match_pattern: &str,
    count: usize,
) -> Result<Vec<Vec<u8>>, RedisCountersError> {
    let route = ScanRoute::Client(client);
    let mut keys = Vec::new();
    collect_scan(&route, match_pattern, count, &mut keys).await?;
    Ok(keys)
}

enum ScanRoute<'route> {
    Client(&'route LiveAsyncClient),
    Node {
        cluster: &'route ClusterConnection,
        host: String,
        port: u16,
    },
}

impl ScanRoute<'_> {
    async fn scan(
        &self,
        cursor: &str,
        match_pattern: &str,
        count: usize,
    ) -> Result<Value, LiveClientError> {
        let mut command = cmd("SCAN");
        command
            .arg(cursor)
            .arg("MATCH")
            .arg(match_pattern)
            .arg("COUNT")
            .arg(count);
        match self {
            Self::Client(client) => client.query(&command).await,
            Self::Node {
                cluster,
                host,
                port,
            } => {
                let mut connection = ClusterConnection::clone(cluster);
                let routing = RoutingInfo::SingleNode(SingleNodeRoutingInfo::ByAddress {
                    host: host.clone(),
                    port: *port,
                });
                connection
                    .route_command(command, routing)
                    .await
                    .map_err(LiveClientError::Redis)
            }
        }
    }
}

async fn collect_scan(
    route: &ScanRoute<'_>,
    match_pattern: &str,
    limit: usize,
    keys: &mut Vec<Vec<u8>>,
) -> Result<(), RedisCountersError> {
    let mut cursor = "0".to_owned();
    loop {
        if keys.len() >= limit {
            return Ok(());
        }
        let reply = route.scan(&cursor, match_pattern, limit).await?;
        let (next_cursor, batch) = scan_reply(reply)?;
        for key in batch {
            if keys.len() >= limit {
                return Ok(());
            }
            keys.push(key);
        }
        if next_cursor == "0" {
            return Ok(());
        }
        cursor = next_cursor;
    }
}

fn scan_reply(reply: Value) -> Result<(String, Vec<Vec<u8>>), RedisCountersError> {
    let unexpected = || RedisCountersError::UnexpectedReply {
        operation: SCAN_OPERATION,
    };
    let RedisReply::Array(parts) = classify(reply) else {
        return Err(unexpected());
    };
    let mut remaining = parts.into_iter();
    let Some(cursor_value) = remaining.next() else {
        return Err(unexpected());
    };
    let Some(batch_value) = remaining.next() else {
        return Err(unexpected());
    };
    if remaining.next().is_some() {
        return Err(unexpected());
    }
    let RedisReply::Bulk(cursor_bytes) = classify(cursor_value) else {
        return Err(unexpected());
    };
    let RedisReply::Array(entries) = classify(batch_value) else {
        return Err(unexpected());
    };
    let cursor = str::from_utf8(&cursor_bytes)
        .map_err(|_source| unexpected())?
        .to_owned();
    let mut keys = Vec::with_capacity(entries.len());
    for entry in entries {
        let RedisReply::Bulk(key) = classify(entry) else {
            return Err(unexpected());
        };
        keys.push(key);
    }
    Ok((cursor, keys))
}

fn cluster_primaries(topology: Value) -> Result<Vec<(String, u16)>, RedisCountersError> {
    let unexpected = || RedisCountersError::UnexpectedReply {
        operation: SCAN_OPERATION,
    };
    let RedisReply::Array(slots) = classify(topology) else {
        return Err(unexpected());
    };
    let mut primaries: Vec<(String, u16)> = Vec::new();
    for slot in slots {
        let (host, port) = slot_primary(slot)?;
        let known = primaries
            .iter()
            .any(|(seen_host, seen_port)| seen_host == &host && *seen_port == port);
        if !known {
            primaries.push((host, port));
        }
    }
    Ok(primaries)
}

fn slot_primary(slot: Value) -> Result<(String, u16), RedisCountersError> {
    let unexpected = || RedisCountersError::UnexpectedReply {
        operation: SCAN_OPERATION,
    };
    let RedisReply::Array(nodes) = classify(slot) else {
        return Err(unexpected());
    };
    let Some(primary) = nodes.into_iter().next() else {
        return Err(unexpected());
    };
    let RedisReply::Array(primary_fields) = classify(primary) else {
        return Err(unexpected());
    };
    let mut fields = primary_fields.into_iter();
    let Some(host_value) = fields.next() else {
        return Err(unexpected());
    };
    let RedisReply::Bulk(host_bytes) = classify(host_value) else {
        return Err(unexpected());
    };
    let Some(port_value) = fields.next() else {
        return Err(unexpected());
    };
    let RedisReply::Int(port_number) = classify(port_value) else {
        return Err(unexpected());
    };
    let host = str::from_utf8(&host_bytes)
        .map_err(|_source| unexpected())?
        .to_owned();
    let port = u16::try_from(port_number).map_err(|_source| unexpected())?;
    Ok((host, port))
}
