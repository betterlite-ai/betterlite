#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::budget_permission_rows::JsonData;
use crate::resource_repository::config::{
    ConfigDecodeError, MAX_CONFIG_VALUE_BYTES, MAX_CONFIG_VALUE_DEPTH, decode_config_json,
};

mod failures;
mod rows;
mod support;

use self::support::{Case, create_defaults_table};

#[tokio::test]
async fn postgres_pinned_rows() {
    let case = Case::new().await;
    create_defaults_table(&case.writer).await;
    rows::raw_model_table(&case).await;
    rows::excluded_model_id(&case).await;
    rows::model_json_columns(&case).await;
    rows::model_nonstring_params(&case).await;
    rows::model_decrypt_valid_plaintext_and_corrupt(&case).await;
    rows::constructor_key_not_read_key(&case).await;
    rows::budget_id_column(&case).await;
    rows::config_missing(&case).await;
    rows::config_create_update_fields(&case).await;
    rows::config_original_return_value(&case).await;
    rows::config_verbatim_string(&case).await;
    rows::credential_verbatim_writes(&case).await;
    rows::credential_upsert_preserves_id(&case).await;
    rows::credential_raw_list(&case).await;
    rows::credential_named_lookup(&case).await;
    rows::falsey_credential_maps(&case).await;
    rows::empty_document_create(&case).await;
    rows::empty_document_write(&case).await;
    rows::partial_update_preserves_columns(&case).await;
    rows::bulk_writes(&case).await;
    case.close().await;
    let split = Case::split().await;
    rows::permission_writer_choice(&split).await;
    rows::config_writer_choice(&split).await;
    split.close().await;
}

#[tokio::test]
async fn postgres_failure_effects() {
    let case = Case::new().await;
    failures::missing_client_before_query(&case).await;
    failures::invalid_model_json(&case).await;
    failures::config_bare_string_read_failure(&case).await;
    failures::credential_missing_name(&case).await;
    failures::model_raw_write_before_publication(&case).await;
    failures::credential_write_before_publication(&case).await;
    failures::credential_redis_absent_noop(&case).await;
    failures::credential_redis_failure_warns_but_succeeds(&case).await;
    failures::injected_publisher_error_after_commit(&case).await;
    failures::read_has_no_publication(&case).await;
    failures::config_upsert_failure(&case).await;
    case.close().await;
}

#[test]
fn config_json_conversion() {
    object_array_scalar_null();
    quoted_string();
    duplicate_last_value();
    malformed_json();
}

fn object_array_scalar_null() {
    assert_eq!(decode_config_json(b"null").unwrap(), JsonData::Null);
    assert_eq!(decode_config_json(b"true").unwrap(), JsonData::Bool(true));
    assert_eq!(decode_config_json(b"7").unwrap(), value("7"));
    assert_eq!(
        decode_config_json(br#""text""#).unwrap(),
        JsonData::String("text".to_owned())
    );
    assert_eq!(
        decode_config_json(br#"[1,null,{"k":true}]"#).unwrap(),
        value(r#"[1,null,{"k":true}]"#)
    );
    assert_eq!(
        decode_config_json(br#"{"a":[1,2],"b":null}"#).unwrap(),
        value(r#"{"a":[1,2],"b":null}"#)
    );
}

fn quoted_string() {
    let brackets = "[".repeat(MAX_CONFIG_VALUE_DEPTH + 1);
    let quoted = format!("\"{brackets}\"");
    assert_eq!(
        decode_config_json(quoted.as_bytes()).unwrap(),
        JsonData::String(brackets)
    );
    let escaped = format!("\"\\\"{}\"", "[".repeat(MAX_CONFIG_VALUE_DEPTH + 1));
    let expected = format!("\"{}", "[".repeat(MAX_CONFIG_VALUE_DEPTH + 1));
    assert_eq!(
        decode_config_json(escaped.as_bytes()).unwrap(),
        JsonData::String(expected)
    );
    assert_eq!(
        decode_config_json(br#""a\"[b""#).unwrap(),
        JsonData::String("a\"[b".to_owned())
    );
}

fn duplicate_last_value() {
    let decoded = decode_config_json(br#"{"k":1,"k":2,"nested":{"a":1,"a":2}}"#).unwrap();
    assert_eq!(decoded, value(r#"{"k":2,"nested":{"a":2}}"#));
}

fn malformed_json() {
    assert!(matches!(
        decode_config_json(b"abc"),
        Err(ConfigDecodeError::Json(_))
    ));
    assert!(matches!(
        decode_config_json(b"{"),
        Err(ConfigDecodeError::Json(_))
    ));
    assert!(matches!(
        decode_config_json(b"{} {}"),
        Err(ConfigDecodeError::Json(_))
    ));
    assert!(matches!(
        decode_config_json(b"\xff"),
        Err(ConfigDecodeError::InvalidUtf8(_))
    ));
    assert!(matches!(
        decode_config_json(b""),
        Err(ConfigDecodeError::Json(_))
    ));
}

#[test]
fn native_config_value_limits() {
    let at_limit = format!("\"{}\"", "a".repeat(MAX_CONFIG_VALUE_BYTES - 2));
    assert_eq!(at_limit.len(), MAX_CONFIG_VALUE_BYTES);
    assert_eq!(
        decode_config_json(at_limit.as_bytes()).unwrap(),
        JsonData::String("a".repeat(MAX_CONFIG_VALUE_BYTES - 2))
    );
    let over = format!("\"{}\"", "a".repeat(MAX_CONFIG_VALUE_BYTES - 1));
    assert!(matches!(
        decode_config_json(over.as_bytes()),
        Err(ConfigDecodeError::TooLarge { len }) if len == MAX_CONFIG_VALUE_BYTES + 1
    ));
    let exact_depth = format!(
        "{}0{}",
        "[".repeat(MAX_CONFIG_VALUE_DEPTH),
        "]".repeat(MAX_CONFIG_VALUE_DEPTH)
    );
    assert!(decode_config_json(exact_depth.as_bytes()).is_ok());
    let over_depth = format!(
        "{}0{}",
        "[".repeat(MAX_CONFIG_VALUE_DEPTH + 1),
        "]".repeat(MAX_CONFIG_VALUE_DEPTH + 1)
    );
    assert!(matches!(
        decode_config_json(over_depth.as_bytes()),
        Err(ConfigDecodeError::TooDeep)
    ));
    let balanced = format!("{}0{}", "[".repeat(40), "]".repeat(40));
    assert!(decode_config_json(balanced.as_bytes()).is_ok());
    assert!(decode_config_json(b"]]]]").is_err());
}

#[rstest]
fn replays_the_resource_config_json_corpus(
    #[files("fuzz/corpus/resource_config_json/*")] path: PathBuf,
) {
    drop(decode_config_json(&fs::read(path).unwrap()));
}

fn value(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}
