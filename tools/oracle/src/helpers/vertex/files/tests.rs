#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

fn run(value: Value) -> Value {
    let input = serde_json::from_value(value).unwrap();
    super::ops::run(&input).unwrap()
}

#[test]
fn helper_seeds_select_vertex_files() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_files"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| {
        validate_helper(Path::new("<helper>"), seed).unwrap();
        serde_json::from_value::<super::ops::Input>(seed.call.input.clone()).unwrap();
        seed.call.operation == "vertex_files"
    }));
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let unknown = serde_json::from_value::<super::ops::Input>(json!({
        "operation": "upload_reply",
        "body": "{}",
        "other": 1_i32,
    }));
    assert!(unknown.unwrap_err().to_string().contains("other"));
}

#[test]
fn an_object_name_follows_the_deployment_model() {
    let value = run(json!({
        "operation": "object_name",
        "file": {"content": "{\"body\":{\"model\":\"gemini-2.0-flash\"}}"},
        "purpose": "batch",
        "deployment_model": "vertex_ai/gemini-2.5-flash",
        "uuid": 7_u64,
    }));
    assert_eq!(
        value,
        json!({"name": "litellm-vertex-files/publishers/google/models/gemini-2.5-flash/00000000-0000-0000-0000-000000000007"})
    );
}

#[test]
fn a_numeric_endpoint_names_an_endpoints_object() {
    let value = run(json!({
        "operation": "object_name",
        "file": {"content": ""},
        "purpose": "batch",
        "deployment_model": "gemini/123456789",
        "passthrough": true,
        "uuid": 7_u64,
    }));
    assert_eq!(
        value,
        json!({"name": "litellm-vertex-files/passthrough/endpoints/123456789/00000000-0000-0000-0000-000000000007"})
    );
}

#[test]
fn a_passthrough_name_needs_the_deployment_model() {
    let input = serde_json::from_value(json!({
        "operation": "object_name",
        "file": {"content": ""},
        "purpose": "batch",
        "passthrough": true,
        "uuid": 1_u64,
    }))
    .unwrap();
    let error = super::ops::run(&input).unwrap_err();
    assert_eq!(error.class, "VertexAIError");
    assert_eq!(error.status, Some(400));
}

#[test]
fn a_batch_custom_endpoint_is_refused_before_the_bucket_lookup() {
    let input = serde_json::from_value(json!({
        "operation": "upload_url",
        "file": {"content": ""},
        "purpose": "batch",
        "custom_endpoint": true,
        "uuid": 1_u64,
    }))
    .unwrap();
    let error = super::ops::run(&input).unwrap_err();
    assert_eq!(error.status, Some(400));
}

#[test]
fn an_upload_url_carries_the_encoded_object_name() {
    let value = run(json!({
        "operation": "upload_url",
        "file": {"content": "", "filename": "batch.jsonl"},
        "purpose": "batch",
        "bucket": "my-bucket/exports",
        "api_base": "https://proxy.example",
        "uuid": 7_u64,
    }));
    assert_eq!(
        value,
        json!({"url": "https://proxy.example/upload/storage/v1/b/my-bucket/o?uploadType=media&name=exports%2Flitellm-vertex-files%2Fuploads%2F00000000000000000000000000000007-batch.jsonl"})
    );
}

#[test]
fn headers_with_an_explicit_key_stay_untouched() {
    let value = run(json!({
        "operation": "headers",
        "headers": {"Authorization": "Bearer caller", "x-test": "yes"},
        "api_key": "explicit",
    }));
    assert_eq!(
        value,
        json!({"headers": {"Authorization": "Bearer caller", "x-test": "yes"}})
    );
}

#[test]
fn an_upload_reply_is_a_file_record() {
    let value = run(json!({
        "operation": "upload_reply",
        "body": "{\"id\":\"my-bucket/litellm-vertex-files/uploads/row/1\",\"name\":\"litellm-vertex-files/uploads/row\",\"size\":\"2\",\"timeCreated\":\"2024-12-04T21:53:12.120184Z\",\"purpose\":\"batch\"}",
    }));
    assert_eq!(
        value,
        json!({"file": {
            "id": "gs://my-bucket/litellm-vertex-files/uploads/row",
            "bytes": 2,
            "created_at": 1733349192,
            "filename": "litellm-vertex-files/uploads/row",
            "object": "file",
            "purpose": "batch",
            "status": "uploaded",
            "expires_at": null,
            "status_details": null,
        }})
    );
}

#[test]
fn a_delete_url_names_the_deleted_object() {
    let value = run(json!({
        "operation": "delete_reply",
        "url": "https://storage.googleapis.com/storage/v1/b/my-bucket/o/litellm-vertex-files%2Fuploads%2Frow",
    }));
    assert_eq!(
        value,
        json!({
            "id": "gs://my-bucket/litellm-vertex-files/uploads/row",
            "object": "file",
            "deleted": true,
        })
    );
}

#[test]
fn an_unusable_delete_url_falls_back_to_deleted() {
    let value = run(json!({"operation": "delete_reply", "url": null}));
    assert_eq!(
        value,
        json!({"id": "deleted", "object": "file", "deleted": true})
    );
}

#[test]
fn listing_is_unsupported() {
    let input = serde_json::from_value(json!({"operation": "list"})).unwrap();
    let error = super::ops::run(&input).unwrap_err();
    assert_eq!(error.class, "NotImplementedError");
    assert_eq!(error.status, None);
    assert_eq!(
        error.message,
        "VertexAIFilesConfig does not support file listing"
    );
}
