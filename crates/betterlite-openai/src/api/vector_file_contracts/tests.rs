#![cfg(test)]

use super::records::{ChunkingKind, FileStatus, ListOrder, VectorStoreFileObject};
use super::wire::{
    VectorFileRecord, VectorFileRecordError, VectorFileRecordKind, WireDecodeLimits,
    decode_vector_file_contracts,
};
use rstest::rstest;
use serde_json::Value;
use std::fs;
use std::path::Path;

fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 0x0001_0000,
        max_items: 4096,
    }
}

fn decode(
    kind: VectorFileRecordKind,
    input: &str,
) -> Result<VectorFileRecord, VectorFileRecordError> {
    decode_vector_file_contracts(kind, input.as_bytes(), limits())
}

fn round_trip(kind: VectorFileRecordKind, input: &str) {
    let record = decode(kind, input).unwrap();
    assert_eq!(
        serde_json::to_value(record).unwrap(),
        serde_json::from_str::<Value>(input).unwrap()
    );
}

#[rstest]
#[case::file(VectorFileRecordKind::FileObject, r#"{"id":"file_a","object":"vector_store.file","created_at":1,"usage_bytes":2,"vector_store_id":"vs_a","status":"in_progress","last_error":{"code":"bad","message":"failed"},"chunking_strategy":{"type":"auto"},"attributes":{"text":"x","integer":2,"decimal":0.25,"boolean":false}}"#)]
#[case::file_null_fields(VectorFileRecordKind::FileObject, r#"{"id":"file_a","usage_bytes":null,"last_error":null,"chunking_strategy":null,"attributes":null}"#)]
#[case::file_empty(VectorFileRecordKind::FileObject, "{}")]
#[case::static_chunking(
    VectorFileRecordKind::ChunkingStrategy,
    r#"{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}}"#
)]
#[case::static_without_config(
    VectorFileRecordKind::ChunkingStrategy,
    r#"{"type":"static","static":null}"#
)]
#[case::unchecked_token_range(
    VectorFileRecordKind::StaticChunking,
    r#"{"max_chunk_size_tokens":-5,"chunk_overlap_tokens":900000}"#
)]
#[case::create(
    VectorFileRecordKind::CreateRequest,
    r#"{"file_id":"file_a","attributes":{},"chunking_strategy":{"type":"auto"}}"#
)]
#[case::create_nulls(
    VectorFileRecordKind::CreateRequest,
    r#"{"attributes":null,"chunking_strategy":null}"#
)]
#[case::update(VectorFileRecordKind::UpdateRequest, r#"{"attributes":{"flag":true}}"#)]
#[case::update_empty(VectorFileRecordKind::UpdateRequest, "{}")]
#[case::list_query(
    VectorFileRecordKind::ListQuery,
    r#"{"after":"","before":"file_b","filter":"failed","limit":0,"order":"asc"}"#
)]
#[case::list_query_nulls(
    VectorFileRecordKind::ListQuery,
    r#"{"after":null,"filter":null,"limit":null,"order":null}"#
)]
#[case::list(VectorFileRecordKind::ListResponse, r#"{"object":"list","data":[{"id":"file_a"}],"first_id":"file_a","last_id":null,"has_more":false}"#)]
#[case::deleted(
    VectorFileRecordKind::DeleteResponse,
    r#"{"id":"file_a","object":"vector_store.file.deleted","deleted":true}"#
)]
#[case::content(VectorFileRecordKind::ContentResponse, r#"{"file_id":"file_a","filename":null,"attributes":{"k":"v"},"content":[{"type":"text","text":"hello"}]}"#)]
#[case::text_part(
    VectorFileRecordKind::ContentTextPart,
    r#"{"type":"text","text":"hello"}"#
)]
#[case::auth(
    VectorFileRecordKind::AuthCredentials,
    r#"{"headers":{"Authorization":"Bearer x","n":1},"query_params":{"p":null}}"#
)]
#[case::auth_missing_maps(VectorFileRecordKind::AuthCredentials, "{}")]
#[case::auth_missing_query(VectorFileRecordKind::AuthCredentials, r#"{"headers":{}}"#)]
#[case::unknown_keys_kept(
    VectorFileRecordKind::FileObject,
    r#"{"id":"file_a","provider_field":{"nested":[1,2]}}"#
)]
fn records_round_trip_exactly(#[case] kind: VectorFileRecordKind, #[case] input: &str) {
    round_trip(kind, input);
}

#[rstest]
#[case::in_progress("in_progress", FileStatus::InProgress)]
#[case::completed("completed", FileStatus::Completed)]
#[case::failed("failed", FileStatus::Failed)]
#[case::cancelled("cancelled", FileStatus::Cancelled)]
fn every_status_literal_is_decoded(#[case] wire: &str, #[case] expected: FileStatus) {
    let input = format!(r#"{{"status":"{wire}"}}"#);
    let file: VectorStoreFileObject = serde_json::from_str(&input).unwrap();
    assert_eq!(file.status, Some(expected));
    round_trip(VectorFileRecordKind::FileObject, &input);
}

#[test]
fn list_order_and_chunking_literals_are_decoded() {
    let VectorFileRecord::ListQuery(query) =
        decode(VectorFileRecordKind::ListQuery, r#"{"order":"desc"}"#).unwrap()
    else {
        panic!("not a list query");
    };
    assert_eq!(query.order, Some(ListOrder::Desc));
    let VectorFileRecord::ChunkingStrategy(strategy) = decode(
        VectorFileRecordKind::ChunkingStrategy,
        r#"{"type":"static"}"#,
    )
    .unwrap() else {
        panic!("not a chunking strategy");
    };
    assert_eq!(strategy.kind, Some(ChunkingKind::Static));
    assert!(strategy.static_config.is_none());
}

#[test]
fn null_is_kept_apart_from_absence() {
    let VectorFileRecord::FileObject(null_file) = decode(
        VectorFileRecordKind::FileObject,
        r#"{"usage_bytes":null,"attributes":null}"#,
    )
    .unwrap() else {
        panic!("not a file");
    };
    let VectorFileRecord::FileObject(absent_file) =
        decode(VectorFileRecordKind::FileObject, "{}").unwrap()
    else {
        panic!("not a file");
    };
    assert!(null_file.usage_bytes.is_none() && null_file.extra.contains_key("usage_bytes"));
    assert!(null_file.attributes.is_none() && null_file.extra.contains_key("attributes"));
    assert!(absent_file.extra.is_empty());
    assert_ne!(null_file, absent_file);
}

#[test]
fn numeric_attributes_keep_their_decimal_digits() {
    let input = r#"{"attributes":{"exact":0.1234567890123456789,"whole":2,"flag":false}}"#;
    round_trip(VectorFileRecordKind::CreateRequest, input);
}

#[rstest]
#[case::attribute_list(VectorFileRecordKind::UpdateRequest, r#"{"attributes":{"k":[1]}}"#)]
#[case::attribute_null_value(VectorFileRecordKind::UpdateRequest, r#"{"attributes":{"k":null}}"#)]
#[case::attribute_object(VectorFileRecordKind::UpdateRequest, r#"{"attributes":{"k":{"a":1}}}"#)]
#[case::attribute_overflow(VectorFileRecordKind::UpdateRequest, r#"{"attributes":{"k":1e99999}}"#)]
#[case::unknown_status(VectorFileRecordKind::FileObject, r#"{"status":"queued"}"#)]
#[case::wrong_object(VectorFileRecordKind::ListResponse, r#"{"object":"collection"}"#)]
#[case::string_limit(VectorFileRecordKind::ListQuery, r#"{"limit":"5"}"#)]
#[case::fractional_count(VectorFileRecordKind::FileObject, r#"{"created_at":1.5}"#)]
#[case::wrong_container(VectorFileRecordKind::ListResponse, r#"{"data":{}}"#)]
#[case::not_an_object(VectorFileRecordKind::FileObject, "[]")]
#[case::truncated(VectorFileRecordKind::FileObject, r#"{"id":"#)]
#[case::empty_input(VectorFileRecordKind::FileObject, "")]
fn invalid_records_are_rejected(#[case] kind: VectorFileRecordKind, #[case] input: &str) {
    assert!(matches!(
        decode(kind, input),
        Err(VectorFileRecordError::Json(_))
    ));
}

#[test]
fn byte_limit_is_checked_before_parsing() {
    let tight = WireDecodeLimits {
        max_json_bytes: 2,
        max_items: 4096,
    };
    let error = decode_vector_file_contracts(VectorFileRecordKind::FileObject, b"{ }  ", tight)
        .unwrap_err();
    assert!(matches!(
        error,
        VectorFileRecordError::JsonTooLarge { limit: 2 }
    ));
    decode_vector_file_contracts(VectorFileRecordKind::FileObject, b"{}", tight).unwrap();
}

#[test]
fn item_limit_counts_every_nested_value() {
    let input = br#"{"attributes":{"a":"x","b":"y"},"id":"f"}"#;
    let exact = WireDecodeLimits {
        max_json_bytes: 1024,
        max_items: 4,
    };
    decode_vector_file_contracts(VectorFileRecordKind::FileObject, input, exact).unwrap();
    let short = WireDecodeLimits {
        max_json_bytes: 1024,
        max_items: 3,
    };
    let error =
        decode_vector_file_contracts(VectorFileRecordKind::FileObject, input, short).unwrap_err();
    assert!(matches!(
        error,
        VectorFileRecordError::TooManyItems { limit: 3 }
    ));
}

#[rstest]
#[case(0, VectorFileRecordKind::FileObject)]
#[case(1, VectorFileRecordKind::CreateRequest)]
#[case(2, VectorFileRecordKind::UpdateRequest)]
#[case(3, VectorFileRecordKind::ListQuery)]
#[case(4, VectorFileRecordKind::ListResponse)]
#[case(5, VectorFileRecordKind::DeleteResponse)]
#[case(6, VectorFileRecordKind::ContentResponse)]
#[case(7, VectorFileRecordKind::ContentTextPart)]
#[case(8, VectorFileRecordKind::AuthCredentials)]
#[case(9, VectorFileRecordKind::ChunkingStrategy)]
#[case(10, VectorFileRecordKind::StaticChunking)]
fn kind_selector_bytes_map_to_kinds(#[case] byte: u8, #[case] expected: VectorFileRecordKind) {
    assert_eq!(VectorFileRecordKind::try_from(byte).unwrap(), expected);
}

#[test]
fn unknown_kind_selector_is_rejected() {
    assert!(matches!(
        VectorFileRecordKind::try_from(11),
        Err(VectorFileRecordError::UnknownKind(11))
    ));
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/vector_file_contracts");
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .filter_map(|bytes| {
            let (&raw_kind, json) = bytes.split_first()?;
            let kind = VectorFileRecordKind::try_from(raw_kind).ok()?;
            Some(decode_vector_file_contracts(kind, json, limits()))
        })
        .count();
    assert!(replayed >= 11);
}
