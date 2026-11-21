use betterlite_config::sections::settings::JsonValue;
use serde::Serialize;

use super::projection::redact_credentials;

// litellm declares field_type as a plain string; these four spellings are its whole catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum FieldType {
    String,
    Integer,
    Boolean,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationRedisSection {
    Connection,
    Cluster,
    Sentinel,
}

// litellm types the default as Any; only a port and three flags carry one, so the marker is all that variant needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum FieldDefault {
    Integer(i64),
    Boolean(bool),
}

// litellm's CoordinationRedisSettingsField: the admin UI's description of one setting, plus the value a read filled in.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CoordinationRedisField {
    pub field_name: &'static str,
    pub field_type: FieldType,
    pub field_value: Option<JsonValue>,
    pub field_description: &'static str,
    pub field_default: Option<FieldDefault>,
    pub ui_field_name: &'static str,
    pub section: CoordinationRedisSection,
}

pub const CATALOG: [CoordinationRedisField; 15] = [
    CoordinationRedisField {
        field_name: "host",
        field_type: FieldType::String,
        field_value: None,
        field_description: "Redis server hostname or IP address",
        field_default: None,
        ui_field_name: "Host",
        section: CoordinationRedisSection::Connection,
    },
    CoordinationRedisField {
        field_name: "port",
        field_type: FieldType::Integer,
        field_value: None,
        field_description: "Redis server port number",
        field_default: Some(FieldDefault::Integer(6379)),
        ui_field_name: "Port",
        section: CoordinationRedisSection::Connection,
    },
    CoordinationRedisField {
        field_name: "username",
        field_type: FieldType::String,
        field_value: None,
        field_description: "Redis server username (if required)",
        field_default: None,
        ui_field_name: "Username",
        section: CoordinationRedisSection::Connection,
    },
    CoordinationRedisField {
        field_name: "password",
        field_type: FieldType::String,
        field_value: None,
        field_description: "Redis server password",
        field_default: None,
        ui_field_name: "Password",
        section: CoordinationRedisSection::Connection,
    },
    CoordinationRedisField {
        field_name: "url",
        field_type: FieldType::String,
        field_value: None,
        field_description: "Full Redis connection URL (e.g. redis://:password@host:6379/1). Set this instead of the discrete host/port/username/password fields.",
        field_default: None,
        ui_field_name: "Redis URL",
        section: CoordinationRedisSection::Connection,
    },
    CoordinationRedisField {
        field_name: "ssl",
        field_type: FieldType::Boolean,
        field_value: None,
        field_description: "Connect to Redis over TLS",
        field_default: Some(FieldDefault::Boolean(false)),
        ui_field_name: "SSL",
        section: CoordinationRedisSection::Connection,
    },
    CoordinationRedisField {
        field_name: "startup_nodes",
        field_type: FieldType::List,
        field_value: None,
        field_description: "Cluster-mode startup nodes (e.g. [{'host': '127.0.0.1', 'port': 7001}]). When set, a Redis Cluster client is used.",
        field_default: None,
        ui_field_name: "Cluster Startup Nodes",
        section: CoordinationRedisSection::Cluster,
    },
    CoordinationRedisField {
        field_name: "sentinel_nodes",
        field_type: FieldType::List,
        field_value: None,
        field_description: "Sentinel [host, port] pairs (e.g. [['localhost', 26379]]). When set, a Sentinel-managed client is used.",
        field_default: None,
        ui_field_name: "Sentinel Nodes",
        section: CoordinationRedisSection::Sentinel,
    },
    CoordinationRedisField {
        field_name: "sentinel_password",
        field_type: FieldType::String,
        field_value: None,
        field_description: "Password for the Redis Sentinel nodes",
        field_default: None,
        ui_field_name: "Sentinel Password",
        section: CoordinationRedisSection::Sentinel,
    },
    CoordinationRedisField {
        field_name: "service_name",
        field_type: FieldType::String,
        field_value: None,
        field_description: "Master service name for Redis Sentinel",
        field_default: None,
        ui_field_name: "Service Name",
        section: CoordinationRedisSection::Sentinel,
    },
    CoordinationRedisField {
        field_name: "aws_iam_auth",
        field_type: FieldType::Boolean,
        field_value: None,
        field_description: "Enable AWS ElastiCache IAM authentication",
        field_default: Some(FieldDefault::Boolean(false)),
        ui_field_name: "AWS IAM Authentication",
        section: CoordinationRedisSection::Connection,
    },
    CoordinationRedisField {
        field_name: "aws_iam_user_name",
        field_type: FieldType::String,
        field_value: None,
        field_description: "AWS ElastiCache IAM user name",
        field_default: None,
        ui_field_name: "AWS IAM User Name",
        section: CoordinationRedisSection::Connection,
    },
    CoordinationRedisField {
        field_name: "aws_iam_cache_name",
        field_type: FieldType::String,
        field_value: None,
        field_description: "AWS ElastiCache cache name",
        field_default: None,
        ui_field_name: "AWS IAM Cache Name",
        section: CoordinationRedisSection::Connection,
    },
    CoordinationRedisField {
        field_name: "aws_iam_region",
        field_type: FieldType::String,
        field_value: None,
        field_description: "AWS region for ElastiCache IAM authentication",
        field_default: None,
        ui_field_name: "AWS IAM Region",
        section: CoordinationRedisSection::Connection,
    },
    CoordinationRedisField {
        field_name: "aws_iam_serverless",
        field_type: FieldType::Boolean,
        field_value: None,
        field_description: "The ElastiCache cache is serverless rather than a self-designed cluster",
        field_default: Some(FieldDefault::Boolean(false)),
        ui_field_name: "AWS IAM Serverless Cache",
        section: CoordinationRedisSection::Connection,
    },
];

// A fresh catalog per call, so one request's filled values never reach the next response.
#[must_use]
pub fn fields(settings: Option<&JsonValue>) -> Vec<CoordinationRedisField> {
    let values = redact_credentials(settings);
    let Some(current) = values.as_object() else {
        return Vec::new();
    };
    CATALOG
        .iter()
        .map(|field| CoordinationRedisField {
            field_value: current.get(field.field_name).cloned(),
            ..field.clone()
        })
        .collect()
}
