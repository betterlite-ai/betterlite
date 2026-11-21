use betterlite_config::sections::settings::JsonValue;
use serde_json::json;

use super::support::object;
use crate::coordination_redis_settings::REDACTED_VALUE;
use crate::coordination_redis_settings::catalog::{CoordinationRedisSection, FieldType, fields};

#[test]
fn the_catalog_lists_the_source_fields_in_order() {
    let listed = fields(None);
    let names: Vec<&str> = listed.iter().map(|field| field.field_name).collect();
    assert_eq!(
        names,
        vec![
            "host",
            "port",
            "username",
            "password",
            "url",
            "ssl",
            "startup_nodes",
            "sentinel_nodes",
            "sentinel_password",
            "service_name",
            "aws_iam_auth",
            "aws_iam_user_name",
            "aws_iam_cache_name",
            "aws_iam_region",
            "aws_iam_serverless",
        ],
    );
}

#[test]
fn the_catalog_carries_the_source_types_and_defaults() {
    let listed = fields(None);
    let port = listed[1].clone();
    assert_eq!(port.field_type, FieldType::Integer);
    assert_eq!(port.field_default, Some(JsonValue::from(6379_i64)));
    assert_eq!(port.section, CoordinationRedisSection::Connection);
    let ssl = listed[5].clone();
    assert_eq!(ssl.field_type, FieldType::Boolean);
    assert_eq!(ssl.field_default, Some(JsonValue::Bool(false)));
    assert_eq!(listed[6].field_type, FieldType::List);
    assert_eq!(listed[7].field_type, FieldType::List);
    assert_eq!(listed[6].section, CoordinationRedisSection::Cluster);
    assert_eq!(listed[7].section, CoordinationRedisSection::Sentinel);
    assert_eq!(listed[10].field_default, Some(JsonValue::Bool(false)));
    assert_eq!(listed[14].field_default, Some(JsonValue::Bool(false)));
    assert_eq!(listed[0].field_default, None);
}

#[test]
fn the_catalog_serializes_with_the_source_spellings() {
    let serialized = serde_json::to_value(fields(None)).expect("the catalog serializes");
    assert_eq!(serialized[0]["field_type"], json!("String"));
    assert_eq!(serialized[1]["field_type"], json!("Integer"));
    assert_eq!(serialized[1]["field_default"], json!(6379_i64));
    assert_eq!(serialized[5]["field_default"], json!(false));
    assert_eq!(serialized[0]["field_default"], JsonValue::Null);
    assert_eq!(serialized[0]["field_value"], JsonValue::Null);
    assert_eq!(serialized[0]["section"], json!("connection"));
    assert_eq!(
        serialized[6]["ui_field_name"],
        json!("Cluster Startup Nodes")
    );
}

#[test]
fn field_values_come_from_the_redacted_settings_only() {
    let settings = object(r#"{"host": "cache.internal", "password": "pw", "port": null}"#);
    let listed = fields(Some(&settings));
    let value_of = |name: &str| {
        listed
            .iter()
            .find(|field| field.field_name == name)
            .cloned()
            .expect("a catalog field")
    };
    assert_eq!(
        value_of("host").field_value,
        Some(JsonValue::String("cache.internal".to_owned())),
    );
    assert_eq!(
        value_of("password").field_value,
        Some(JsonValue::String(REDACTED_VALUE.to_owned())),
    );
    assert_eq!(value_of("port").field_value, Some(JsonValue::Null));
    assert_eq!(value_of("username").field_value, None);
}

#[test]
fn a_new_catalog_is_built_per_call() {
    let filled = fields(Some(&object(r#"{"host": "cache.internal"}"#)));
    assert!(filled.iter().any(|field| field.field_value.is_some()));
    let empty = fields(None);
    assert!(empty.iter().all(|field| field.field_value.is_none()));
    assert_eq!(empty.len(), 15);
}
