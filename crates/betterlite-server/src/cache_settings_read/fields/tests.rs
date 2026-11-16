#![cfg(test)]

use serde_json::Value;

use super::{REDIS_TYPE_DESCRIPTIONS, catalog};
use crate::cache_settings_read::test_support::parse;

const FIELD_MEMBERS: [&str; 9] = [
    "field_name",
    "field_type",
    "field_value",
    "field_description",
    "field_default",
    "options",
    "ui_field_name",
    "link",
    "redis_type",
];

// LiteLLM's CACHE_SETTINGS_FIELDS through model_dump at the pinned commit, one field per line.
const LITELLM_CATALOG: &str = r#"[
{"field_name": "redis_type", "field_type": "String", "field_value": null, "field_description": "Type of Redis deployment", "field_default": "node", "options": ["node", "cluster", "sentinel"], "ui_field_name": "Redis Type", "link": null, "redis_type": null},
{"field_name": "url", "field_type": "String", "field_value": null, "field_description": "Full Redis/Valkey connection URL (e.g. redis://:password@host:6379/1). When set, it takes precedence over Host, Port, Username, Password, and Database Index.", "field_default": null, "options": null, "ui_field_name": "Redis URL", "link": null, "redis_type": null},
{"field_name": "host", "field_type": "String", "field_value": null, "field_description": "Redis server hostname or IP address", "field_default": null, "options": null, "ui_field_name": "Host", "link": null, "redis_type": null},
{"field_name": "port", "field_type": "String", "field_value": null, "field_description": "Redis server port number", "field_default": "6379", "options": null, "ui_field_name": "Port", "link": null, "redis_type": null},
{"field_name": "db", "field_type": "Integer", "field_value": null, "field_description": "Logical database index to isolate the cache (e.g. 1 for redis://host:6379/1)", "field_default": null, "options": null, "ui_field_name": "Database Index", "link": null, "redis_type": null},
{"field_name": "password", "field_type": "String", "field_value": null, "field_description": "Redis server password", "field_default": null, "options": null, "ui_field_name": "Password", "link": null, "redis_type": null},
{"field_name": "username", "field_type": "String", "field_value": null, "field_description": "Redis server username (if required)", "field_default": null, "options": null, "ui_field_name": "Username", "link": null, "redis_type": null},
{"field_name": "ssl", "field_type": "Boolean", "field_value": null, "field_description": "Enable SSL/TLS connection", "field_default": false, "options": null, "ui_field_name": "SSL", "link": null, "redis_type": null},
{"field_name": "namespace", "field_type": "String", "field_value": null, "field_description": "Namespace prefix for cache keys", "field_default": null, "options": null, "ui_field_name": "Namespace", "link": null, "redis_type": null},
{"field_name": "ttl", "field_type": "Float", "field_value": null, "field_description": "Time-to-live for cached items in seconds", "field_default": null, "options": null, "ui_field_name": "TTL (seconds)", "link": null, "redis_type": null},
{"field_name": "max_connections", "field_type": "Integer", "field_value": null, "field_description": "Maximum number of connections in the connection pool", "field_default": null, "options": null, "ui_field_name": "Max Connections", "link": null, "redis_type": null},
{"field_name": "redis_startup_nodes", "field_type": "List", "field_value": null, "field_description": "List of startup nodes for Redis Cluster (e.g., [{'host': '127.0.0.1', 'port': '7001'}])", "field_default": null, "options": null, "ui_field_name": "Startup Nodes", "link": null, "redis_type": "cluster"},
{"field_name": "sentinel_nodes", "field_type": "List", "field_value": null, "field_description": "List of Sentinel nodes (e.g., [['localhost', 26379]])", "field_default": null, "options": null, "ui_field_name": "Sentinel Nodes", "link": null, "redis_type": "sentinel"},
{"field_name": "service_name", "field_type": "String", "field_value": null, "field_description": "Master service name for Redis Sentinel", "field_default": null, "options": null, "ui_field_name": "Service Name", "link": null, "redis_type": "sentinel"},
{"field_name": "sentinel_password", "field_type": "String", "field_value": null, "field_description": "Password for Redis Sentinel authentication", "field_default": null, "options": null, "ui_field_name": "Sentinel Password", "link": null, "redis_type": "sentinel"},
{"field_name": "similarity_threshold", "field_type": "Float", "field_value": null, "field_description": "Similarity threshold for semantic cache", "field_default": 0.8, "options": null, "ui_field_name": "Similarity Threshold", "link": null, "redis_type": "semantic"},
{"field_name": "redis_semantic_cache_embedding_model", "field_type": "Models_Select", "field_value": null, "field_description": "Embedding model for semantic cache", "field_default": null, "options": null, "ui_field_name": "Embedding Model", "link": null, "redis_type": "semantic"},
{"field_name": "semantic_cache_scope", "field_type": "String", "field_value": null, "field_description": "Isolation granularity for semantic cache hits. 'key' shares hits between all end users of a key/team/org. 'end_user' also isolates per end user; requests without an end user fall back to the key scope.", "field_default": "key", "options": ["key", "end_user"], "ui_field_name": "Semantic Cache Scope", "link": null, "redis_type": "semantic"},
{"field_name": "gcp_service_account", "field_type": "String", "field_value": null, "field_description": "GCP service account for IAM authentication (e.g., projects/-/serviceAccounts/your-sa@project.iam.gserviceaccount.com)", "field_default": null, "options": null, "ui_field_name": "GCP Service Account", "link": null, "redis_type": null},
{"field_name": "gcp_ssl_ca_certs", "field_type": "String", "field_value": null, "field_description": "Path to SSL CA certificate file for GCP Memorystore Redis", "field_default": null, "options": null, "ui_field_name": "GCP SSL CA Certs", "link": null, "redis_type": null},
{"field_name": "ssl_cert_reqs", "field_type": "String", "field_value": null, "field_description": "SSL certificate requirements (None, CERT_REQUIRED, CERT_OPTIONAL)", "field_default": null, "options": null, "ui_field_name": "SSL Cert Reqs", "link": null, "redis_type": null},
{"field_name": "ssl_check_hostname", "field_type": "Boolean", "field_value": null, "field_description": "Enable SSL hostname verification", "field_default": null, "options": null, "ui_field_name": "SSL Check Hostname", "link": null, "redis_type": null},
{"field_name": "aws_iam_auth", "field_type": "Boolean", "field_value": null, "field_description": "Enable AWS ElastiCache IAM authentication", "field_default": false, "options": null, "ui_field_name": "AWS IAM Authentication", "link": null, "redis_type": null},
{"field_name": "aws_iam_user_name", "field_type": "String", "field_value": null, "field_description": "AWS ElastiCache IAM user name", "field_default": null, "options": null, "ui_field_name": "AWS IAM User Name", "link": null, "redis_type": null},
{"field_name": "aws_iam_cache_name", "field_type": "String", "field_value": null, "field_description": "AWS ElastiCache cache name", "field_default": null, "options": null, "ui_field_name": "AWS IAM Cache Name", "link": null, "redis_type": null},
{"field_name": "aws_iam_region", "field_type": "String", "field_value": null, "field_description": "AWS region for ElastiCache IAM authentication", "field_default": null, "options": null, "ui_field_name": "AWS IAM Region", "link": null, "redis_type": null},
{"field_name": "aws_iam_serverless", "field_type": "Boolean", "field_value": null, "field_description": "The ElastiCache cache is serverless rather than a self-designed cluster", "field_default": false, "options": null, "ui_field_name": "AWS IAM Serverless Cache", "link": null, "redis_type": null}
]"#;

const LITELLM_DESCRIPTIONS: &str = r#"{"node": "Standard Redis node/single instance", "cluster": "Redis Cluster mode for high availability and horizontal scaling", "sentinel": "Redis Sentinel mode for high availability with automatic failover"}"#;

#[test]
fn the_catalog_matches_litellms_cache_settings_fields() {
    let actual = serde_json::to_value(catalog()).expect("catalog");
    assert_eq!(actual, parse(LITELLM_CATALOG));
}

#[test]
fn the_redis_type_descriptions_carry_litellms_three_texts() {
    let actual = serde_json::to_value(REDIS_TYPE_DESCRIPTIONS).expect("descriptions");
    assert_eq!(actual, parse(LITELLM_DESCRIPTIONS));
}

#[test]
fn every_field_serializes_all_nine_members_with_unset_ones_null() {
    let serialized = serde_json::to_value(catalog()).expect("catalog");
    let fields: Vec<_> = serialized
        .as_array()
        .expect("the catalog list")
        .iter()
        .map(|field| field.as_object().expect("a field object"))
        .collect();
    assert!(fields.iter().all(|field| field.len() == 9));
    assert!(
        fields
            .iter()
            .all(|field| FIELD_MEMBERS.iter().all(|name| field.contains_key(*name)))
    );
    assert!(
        fields
            .iter()
            .all(|field| field.get("field_value") == Some(&Value::Null))
    );
}

#[test]
fn each_call_returns_an_independent_catalog() {
    let mut filled = catalog();
    filled[0].field_value = Some(Value::String("filled".to_owned()));
    let fresh = catalog();
    assert_eq!(fresh[0].field_value, None);
    assert_eq!(
        filled[0].field_value,
        Some(Value::String("filled".to_owned()))
    );
}
