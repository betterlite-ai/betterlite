use betterlite_config::sections::settings::JsonValue;
use serde::Serialize;

use super::projection::{Redaction, redact};

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

// The only defaults litellm's catalog presents; a Value number cannot be built in a constant, so a read converts them.
#[derive(Clone, Copy, Debug)]
enum PresentedDefault {
    Port,
    False,
}

// litellm's CoordinationRedisSettingsField: the admin UI's description of one setting, plus the value a read filled in.
#[derive(Clone, Debug, Serialize)]
pub struct CoordinationRedisField {
    pub field_name: &'static str,
    pub field_type: FieldType,
    pub field_value: Option<JsonValue>,
    pub field_description: &'static str,
    pub field_default: Option<JsonValue>,
    pub ui_field_name: &'static str,
    pub section: CoordinationRedisSection,
}

// The constant half of a row: litellm's field metadata without the value a read fills in.
struct CatalogEntry {
    field_name: &'static str,
    field_type: FieldType,
    field_description: &'static str,
    default: Option<PresentedDefault>,
    ui_field_name: &'static str,
    section: CoordinationRedisSection,
}

const PORT: i64 = 6379;

const CATALOG: [CatalogEntry; 15] = [
    CatalogEntry {
        field_name: "host",
        field_type: FieldType::String,
        field_description: "Redis server hostname or IP address",
        default: None,
        ui_field_name: "Host",
        section: CoordinationRedisSection::Connection,
    },
    CatalogEntry {
        field_name: "port",
        field_type: FieldType::Integer,
        field_description: "Redis server port number",
        default: Some(PresentedDefault::Port),
        ui_field_name: "Port",
        section: CoordinationRedisSection::Connection,
    },
    CatalogEntry {
        field_name: "username",
        field_type: FieldType::String,
        field_description: "Redis server username (if required)",
        default: None,
        ui_field_name: "Username",
        section: CoordinationRedisSection::Connection,
    },
    CatalogEntry {
        field_name: "password",
        field_type: FieldType::String,
        field_description: "Redis server password",
        default: None,
        ui_field_name: "Password",
        section: CoordinationRedisSection::Connection,
    },
    CatalogEntry {
        field_name: "url",
        field_type: FieldType::String,
        field_description: "Full Redis connection URL (e.g. redis://:password@host:6379/1). Set this instead of the discrete host/port/username/password fields.",
        default: None,
        ui_field_name: "Redis URL",
        section: CoordinationRedisSection::Connection,
    },
    CatalogEntry {
        field_name: "ssl",
        field_type: FieldType::Boolean,
        field_description: "Connect to Redis over TLS",
        default: Some(PresentedDefault::False),
        ui_field_name: "SSL",
        section: CoordinationRedisSection::Connection,
    },
    CatalogEntry {
        field_name: "startup_nodes",
        field_type: FieldType::List,
        field_description: "Cluster-mode startup nodes (e.g. [{'host': '127.0.0.1', 'port': 7001}]). When set, a Redis Cluster client is used.",
        default: None,
        ui_field_name: "Cluster Startup Nodes",
        section: CoordinationRedisSection::Cluster,
    },
    CatalogEntry {
        field_name: "sentinel_nodes",
        field_type: FieldType::List,
        field_description: "Sentinel [host, port] pairs (e.g. [['localhost', 26379]]). When set, a Sentinel-managed client is used.",
        default: None,
        ui_field_name: "Sentinel Nodes",
        section: CoordinationRedisSection::Sentinel,
    },
    CatalogEntry {
        field_name: "sentinel_password",
        field_type: FieldType::String,
        field_description: "Password for the Redis Sentinel nodes",
        default: None,
        ui_field_name: "Sentinel Password",
        section: CoordinationRedisSection::Sentinel,
    },
    CatalogEntry {
        field_name: "service_name",
        field_type: FieldType::String,
        field_description: "Master service name for Redis Sentinel",
        default: None,
        ui_field_name: "Service Name",
        section: CoordinationRedisSection::Sentinel,
    },
    CatalogEntry {
        field_name: "aws_iam_auth",
        field_type: FieldType::Boolean,
        field_description: "Enable AWS ElastiCache IAM authentication",
        default: Some(PresentedDefault::False),
        ui_field_name: "AWS IAM Authentication",
        section: CoordinationRedisSection::Connection,
    },
    CatalogEntry {
        field_name: "aws_iam_user_name",
        field_type: FieldType::String,
        field_description: "AWS ElastiCache IAM user name",
        default: None,
        ui_field_name: "AWS IAM User Name",
        section: CoordinationRedisSection::Connection,
    },
    CatalogEntry {
        field_name: "aws_iam_cache_name",
        field_type: FieldType::String,
        field_description: "AWS ElastiCache cache name",
        default: None,
        ui_field_name: "AWS IAM Cache Name",
        section: CoordinationRedisSection::Connection,
    },
    CatalogEntry {
        field_name: "aws_iam_region",
        field_type: FieldType::String,
        field_description: "AWS region for ElastiCache IAM authentication",
        default: None,
        ui_field_name: "AWS IAM Region",
        section: CoordinationRedisSection::Connection,
    },
    CatalogEntry {
        field_name: "aws_iam_serverless",
        field_type: FieldType::Boolean,
        field_description: "The ElastiCache cache is serverless rather than a self-designed cluster",
        default: Some(PresentedDefault::False),
        ui_field_name: "AWS IAM Serverless Cache",
        section: CoordinationRedisSection::Connection,
    },
];

// litellm carries these as an object default: the port number and false for the three IAM/TLS flags.
fn presented_default(default: Option<PresentedDefault>) -> Option<JsonValue> {
    match default {
        Some(PresentedDefault::Port) => Some(JsonValue::from(PORT)),
        Some(PresentedDefault::False) => Some(JsonValue::Bool(false)),
        None => None,
    }
}

// A fresh catalog per call, so one request's filled values never reach the next response.
#[must_use]
pub fn fields(settings: Option<&JsonValue>) -> Vec<CoordinationRedisField> {
    let values = redact(settings, Redaction::Credentials);
    let Some(current) = values.as_object() else {
        return Vec::new();
    };
    CATALOG
        .iter()
        .map(|entry| CoordinationRedisField {
            field_name: entry.field_name,
            field_type: entry.field_type,
            field_value: current.get(entry.field_name).cloned(),
            field_description: entry.field_description,
            field_default: presented_default(entry.default),
            ui_field_name: entry.ui_field_name,
            section: entry.section,
        })
        .collect()
}
