#![cfg(test)]

use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use rstest::rstest;
use uuid::Uuid;

use super::get_object_name;
use crate::vertex_files::FileRequest;
use crate::vertex_files::error::VertexFilesError;

const ID: u128 = 7;
const HYPHENATED: &str = "00000000-0000-0000-0000-000000000007";
const SIMPLE: &str = "00000000000000000000000000000007";

fn jsonl(lines: &[&str]) -> FileData {
    FileData::Tuple2 {
        filename: Some("batch.jsonl".to_owned()),
        content: FileBody::Bytes(lines.join("\n").into_bytes()),
    }
}

#[test]
fn the_deployment_model_wins_over_the_jsonl_body_model() {
    let file = jsonl(&[r#"{"custom_id":"a","body":{"model":"gemini-2.5-pro"}}"#]);
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: Some("vertex_ai/gemini-2.5-flash"),
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!("litellm-vertex-files/publishers/google/models/gemini-2.5-flash/{HYPHENATED}")
    );
}

#[test]
fn a_direct_upload_takes_the_jsonl_body_model() {
    let file = jsonl(&[r#"{"body":{"model":"gemini-2.0-flash"}}"#]);
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!("litellm-vertex-files/publishers/google/models/gemini-2.0-flash/{HYPHENATED}")
    );
}

#[test]
fn an_empty_deployment_model_falls_back_to_the_jsonl_body_model() {
    let file = jsonl(&[r#"{"body":{"model":"gemini-2.0-flash"}}"#]);
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: Some(""),
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!("litellm-vertex-files/publishers/google/models/gemini-2.0-flash/{HYPHENATED}")
    );
}

#[test]
fn a_body_model_keeps_its_provider_prefix() {
    let file = jsonl(&[r#"{"body":{"model":"vertex_ai/gemini-2.0-flash"}}"#]);
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!(
            "litellm-vertex-files/publishers/google/models/vertex_ai/gemini-2.0-flash/{HYPHENATED}"
        )
    );
}

#[rstest]
#[case::bare_id("123456789")]
#[case::vertex_bare_id("vertex_ai/123456789")]
#[case::vertex_gemini_id("vertex_ai/gemini/123456789")]
#[case::gemini_id("gemini/123456789")]
fn a_numeric_endpoint_model_uses_the_endpoints_path(#[case] model: &str) {
    let file = jsonl(&[r#"{"body":{"model":"ignored"}}"#]);
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: Some(model),
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!("litellm-vertex-files/endpoints/123456789/{HYPHENATED}")
    );
}

#[test]
fn an_embedded_publishers_path_is_kept() {
    let file = jsonl(&[r#"{"body":{"model":"ignored"}}"#]);
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: Some("vertex_ai/publishers/google/models/gemini-2.5-pro"),
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!("litellm-vertex-files/publishers/google/models/gemini-2.5-pro/{HYPHENATED}")
    );
}

#[test]
fn a_bare_model_gets_the_publishers_prefix() {
    let file = jsonl(&[r#"{"body":{"model":"ignored"}}"#]);
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: Some("gemini-2.5-flash"),
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!("litellm-vertex-files/publishers/google/models/gemini-2.5-flash/{HYPHENATED}")
    );
}

#[test]
fn the_model_path_is_sanitized() {
    let file = jsonl(&[r#"{"body":{"model":"ignored"}}"#]);
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: Some("gemini 2.5/pro"),
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!("litellm-vertex-files/publishers/google/models/gemini_2.5/pro/{HYPHENATED}")
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn passthrough_requires_the_deployment_model(#[case] model: Option<&str>) {
    let file = jsonl(&[r#"{"body":{"model":"gemini-2.0-flash"}}"#]);
    let failure = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: model,
            passthrough: true,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .expect_err("a passthrough upload has no row to name the object from");
    assert!(matches!(failure, VertexFilesError::PassthroughModel));
    assert_eq!(failure.status_code(), Some(400));
}

#[test]
fn passthrough_uses_its_own_prefix() {
    let file = jsonl(&[r#"{"body":{"model":"ignored"}}"#]);
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: Some("vertex_ai/gemini-2.5-pro"),
            passthrough: true,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!(
            "litellm-vertex-files/passthrough/publishers/google/models/gemini-2.5-pro/{HYPHENATED}"
        )
    );
}

#[test]
fn passthrough_never_probes_the_jsonl() {
    let file = FileData::Direct(FileBody::Bytes(b"{not json".to_vec()));
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: Some("gemini-2.5-pro"),
            passthrough: true,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!(
            "litellm-vertex-files/passthrough/publishers/google/models/gemini-2.5-pro/{HYPHENATED}"
        )
    );
}

#[test]
fn passthrough_only_names_batch_uploads() {
    let file = FileData::Direct(FileBody::Bytes(b"not json".to_vec()));
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::FineTune,
            deployment_model: Some("gemini-2.5-pro"),
            passthrough: true,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(name, format!("litellm-vertex-files/uploads/{SIMPLE}-file"));
}

#[test]
fn an_empty_batch_file_uses_the_uploads_prefix() {
    let file = jsonl(&["", "  "]);
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!("litellm-vertex-files/uploads/{SIMPLE}-batch.jsonl")
    );
}

#[test]
fn the_uploads_filename_is_sanitized() {
    let file = FileData::Tuple2 {
        filename: Some("../../etc/passwd".to_owned()),
        content: FileBody::Bytes(b"data".to_vec()),
    };
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Assistants,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!("litellm-vertex-files/uploads/{SIMPLE}-passwd")
    );
}

#[test]
fn a_file_without_a_name_falls_back_to_file() {
    let file = FileData::Direct(FileBody::Bytes(b"data".to_vec()));
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Messages,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(name, format!("litellm-vertex-files/uploads/{SIMPLE}-file"));
}

#[test]
fn a_batch_entry_without_a_model_uses_the_empty_model() {
    let lines = [
        r#"{"custom_id":"a"}"#,
        r#"{"body":{}}"#,
        r#"{"body":null}"#,
        r#"{"body":{"model":null}}"#,
    ];
    let names: Vec<String> = lines
        .iter()
        .map(|line| {
            let file = jsonl(&[*line]);
            get_object_name(
                &FileRequest {
                    file: &file,
                    purpose: CreatePurpose::Batch,
                    deployment_model: None,
                    passthrough: false,
                    custom_endpoint: false,
                },
                Uuid::from_u128(ID),
            )
            .unwrap()
        })
        .collect();
    assert_eq!(
        names,
        vec![format!("litellm-vertex-files/publishers/google/models/{HYPHENATED}"); lines.len()]
    );
}

#[test]
fn a_blank_first_line_is_skipped() {
    let file = jsonl(&["", "   ", r#"{"body":{"model":"gemini-2.0-flash"}}"#]);
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        name,
        format!("litellm-vertex-files/publishers/google/models/gemini-2.0-flash/{HYPHENATED}")
    );
}

#[test]
fn a_non_object_body_is_refused() {
    let file = jsonl(&[r#"{"body":"text"}"#]);
    let failure = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .expect_err("a body that is not an object has no model");
    assert!(matches!(failure, VertexFilesError::BatchBody));
}

#[test]
fn a_non_text_model_is_refused() {
    let file = jsonl(&[r#"{"body":{"model":7}}"#]);
    let failure = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .expect_err("a model that is not text cannot name an object");
    assert!(matches!(failure, VertexFilesError::BatchModel));
}

#[test]
fn a_malformed_batch_line_is_refused() {
    let file = jsonl(&["{not json"]);
    let failure = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .expect_err("json.loads fails on the first line");
    assert!(matches!(failure, VertexFilesError::Line(_)));
}

#[test]
fn a_non_utf8_batch_file_is_refused() {
    let file = FileData::Direct(FileBody::Bytes(vec![0xff, 0xfe]));
    let failure = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .expect_err("a batch file is JSONL text");
    assert!(matches!(failure, VertexFilesError::ContentUtf8(_)));
}

#[test]
fn a_non_batch_purpose_never_probes_the_file() {
    let file = FileData::Direct(FileBody::Bytes(b"\xff{not json".to_vec()));
    let name = get_object_name(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::FineTune,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(name, format!("litellm-vertex-files/uploads/{SIMPLE}-file"));
}
