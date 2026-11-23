#![cfg(test)]

use betterlite_models::decisions::Json;
use serde_json::Number;

use super::{
    BLOB_KEY, CONTENT_TYPE_KEY, CallFile, CallFilesError, FILE_KEY, NAME_KEY, files_from_payload,
};
use crate::call_setup::RequestMap;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn count(value: u64) -> Json {
    Json::Number(Number::from(value))
}

fn object(fields: Vec<(&str, Json)>) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

// The Python side's marker: the file's name, the index of its bytes and its type, any of which can be absent.
fn marker(name: Option<&str>, blob: Option<u64>, content_type: Option<&str>) -> Json {
    let mut fields = Vec::new();
    if let Some(given) = name {
        fields.push((NAME_KEY, text(given)));
    }
    if let Some(index) = blob {
        fields.push((BLOB_KEY, count(index)));
    }
    if let Some(kind) = content_type {
        fields.push((CONTENT_TYPE_KEY, text(kind)));
    }
    object(vec![(FILE_KEY, object(fields))])
}

fn payload(entries: Vec<(&str, Json)>) -> RequestMap {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

#[test]
fn call_files_are_collected_in_key_order() {
    let request = payload(vec![
        ("image", marker(Some("cat.png"), Some(0), Some("image/png"))),
        ("mask", marker(Some("mask.png"), Some(1), None)),
    ]);
    let mut blobs = vec![b"cat".to_vec(), b"mask".to_vec()];

    let files = files_from_payload(&request, &mut blobs).unwrap();

    assert_eq!(
        files,
        vec![
            (
                "image".to_owned(),
                CallFile {
                    name: "cat.png".to_owned(),
                    content_type: Some("image/png".to_owned()),
                    bytes: b"cat".to_vec(),
                },
            ),
            (
                "mask".to_owned(),
                CallFile {
                    name: "mask.png".to_owned(),
                    content_type: None,
                    bytes: b"mask".to_vec(),
                },
            ),
        ]
    );
}

#[test]
fn call_files_are_found_below_lists_and_objects() {
    let images = Json::List(vec![
        marker(Some("one.png"), Some(0), None),
        marker(Some("two.png"), Some(1), None),
    ]);
    let request = payload(vec![("batch", object(vec![("images", images)]))]);
    let mut blobs = vec![b"one".to_vec(), b"two".to_vec()];

    let files = files_from_payload(&request, &mut blobs).unwrap();

    let keys: Vec<&str> = files.iter().map(|(key, _file)| key.as_str()).collect();
    let names: Vec<&str> = files
        .iter()
        .map(|(_key, file)| file.name.as_str())
        .collect();
    assert_eq!(keys, vec!["images", "images"]);
    assert_eq!(names, vec!["one.png", "two.png"]);
}

#[test]
fn call_files_default_a_missing_name() {
    let request = payload(vec![("image", marker(None, Some(0), None))]);
    let mut blobs = vec![b"cat".to_vec()];

    let files = files_from_payload(&request, &mut blobs).unwrap();

    assert_eq!(files.len(), 1);
    let (_key, file) = files.into_iter().next().unwrap();
    assert_eq!(file.name, "file");
    assert_eq!(file.content_type, None);
}

#[test]
fn call_files_read_a_null_name_as_the_default() {
    let request = payload(vec![(
        "image",
        object(vec![(
            FILE_KEY,
            object(vec![(NAME_KEY, Json::Null), (BLOB_KEY, count(0))]),
        )]),
    )]);
    let mut blobs = vec![b"cat".to_vec()];

    let files = files_from_payload(&request, &mut blobs).unwrap();

    assert_eq!(files.len(), 1);
    let (_key, file) = files.into_iter().next().unwrap();
    assert_eq!(file.name, "file");
}

#[test]
fn call_files_read_a_null_content_type_as_none() {
    let request = payload(vec![(
        "mask",
        object(vec![(
            FILE_KEY,
            object(vec![(BLOB_KEY, count(0)), (CONTENT_TYPE_KEY, Json::Null)]),
        )]),
    )]);
    let mut blobs = vec![b"mask".to_vec()];

    let files = files_from_payload(&request, &mut blobs).unwrap();

    assert_eq!(files.len(), 1);
    let (_key, file) = files.into_iter().next().unwrap();
    assert_eq!(file.content_type, None);
}

#[test]
fn call_files_ignore_plain_keys_beside_the_marker() {
    let marked = object(vec![
        ("extra", text("x")),
        (FILE_KEY, object(vec![(BLOB_KEY, count(0))])),
    ]);
    let request = payload(vec![("image", marked)]);
    let mut blobs = vec![b"cat".to_vec()];

    let files = files_from_payload(&request, &mut blobs).unwrap();

    assert_eq!(files.len(), 1);
    let (key, file) = files.into_iter().next().unwrap();
    assert_eq!(key, "image");
    assert_eq!(file.bytes, b"cat".to_vec());
}

#[test]
fn call_files_move_the_blob_out_of_the_list() {
    let request = payload(vec![("image", marker(Some("cat.png"), Some(0), None))]);
    let mut blobs = vec![b"cat".to_vec()];

    let files = files_from_payload(&request, &mut blobs).unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(blobs, vec![Vec::<u8>::new()]);
}

#[test]
fn call_files_leave_plain_values_alone() {
    let request = payload(vec![
        ("model", text("openai/gpt-4o")),
        ("nested", Json::List(vec![text("a"), Json::Null, count(2)])),
        ("flags", object(vec![("stream", Json::Bool(true))])),
    ]);
    let mut blobs: Vec<Vec<u8>> = Vec::new();

    let files = files_from_payload(&request, &mut blobs).unwrap();

    assert!(files.is_empty());
}

#[test]
fn call_files_refuse_a_file_with_no_blob() {
    let request = payload(vec![("image", marker(Some("cat.png"), None, None))]);
    let mut blobs: Vec<Vec<u8>> = Vec::new();

    let error = files_from_payload(&request, &mut blobs).unwrap_err();

    assert!(matches!(error, CallFilesError::MissingBlob { key } if key == "image"));
}

#[test]
fn call_files_refuse_a_null_blob() {
    let request = payload(vec![(
        "image",
        object(vec![(
            FILE_KEY,
            object(vec![(BLOB_KEY, Json::Null), (NAME_KEY, text("cat.png"))]),
        )]),
    )]);
    let mut blobs = vec![b"cat".to_vec()];

    let error = files_from_payload(&request, &mut blobs).unwrap_err();

    assert!(matches!(error, CallFilesError::MissingBlob { key } if key == "image"));
}

#[test]
fn call_files_refuse_a_blob_index_that_is_not_a_number() {
    let request = payload(vec![(
        "image",
        object(vec![(FILE_KEY, object(vec![(BLOB_KEY, text("0"))]))]),
    )]);
    let mut blobs = vec![b"cat".to_vec()];

    let error = files_from_payload(&request, &mut blobs).unwrap_err();

    assert!(matches!(error, CallFilesError::InvalidIndex { key } if key == "image"));
}

#[test]
fn call_files_refuse_a_negative_blob_index() {
    let request = payload(vec![(
        "image",
        object(vec![(
            FILE_KEY,
            object(vec![(BLOB_KEY, Json::Number(Number::from(-1_i64)))]),
        )]),
    )]);
    let mut blobs = vec![b"cat".to_vec()];

    let error = files_from_payload(&request, &mut blobs).unwrap_err();

    assert!(matches!(error, CallFilesError::InvalidIndex { key } if key == "image"));
}

#[test]
fn call_files_refuse_a_blob_the_call_did_not_send() {
    let request = payload(vec![("image", marker(Some("cat.png"), Some(2), None))]);
    let mut blobs = vec![b"cat".to_vec()];

    let error = files_from_payload(&request, &mut blobs).unwrap_err();

    assert!(matches!(error, CallFilesError::UnknownBlob { key, index: 2 } if key == "image"));
}

#[test]
fn call_files_refuse_a_huge_blob_index() {
    let request = payload(vec![(
        "image",
        marker(Some("cat.png"), Some(u64::MAX), None),
    )]);
    let mut blobs = vec![b"cat".to_vec()];

    let error = files_from_payload(&request, &mut blobs).unwrap_err();

    assert!(matches!(
        error,
        CallFilesError::UnknownBlob { key, index } if key == "image" && index == u64::MAX
    ));
}

#[test]
fn call_files_refuse_a_blob_used_twice() {
    let request = payload(vec![
        ("image", marker(Some("cat.png"), Some(0), None)),
        ("mask", marker(Some("mask.png"), Some(0), None)),
    ]);
    let mut blobs = vec![b"cat".to_vec()];

    let error = files_from_payload(&request, &mut blobs).unwrap_err();

    assert!(matches!(
        error,
        CallFilesError::DuplicateBlob { key, index: 0 } if key == "mask"
    ));
}

#[test]
fn call_files_refuse_a_marker_that_is_not_an_object() {
    let request = payload(vec![("image", object(vec![(FILE_KEY, text("cat.png"))]))]);
    let mut blobs = vec![b"cat".to_vec()];

    let error = files_from_payload(&request, &mut blobs).unwrap_err();

    assert!(matches!(error, CallFilesError::InvalidMarker { key } if key == "image"));
}

#[test]
fn call_files_refuse_a_name_that_is_not_text() {
    let request = payload(vec![(
        "image",
        object(vec![(
            FILE_KEY,
            object(vec![(NAME_KEY, count(7)), (BLOB_KEY, count(0))]),
        )]),
    )]);
    let mut blobs = vec![b"cat".to_vec()];

    let error = files_from_payload(&request, &mut blobs).unwrap_err();

    assert!(matches!(error, CallFilesError::InvalidMarker { key } if key == "image"));
}

#[test]
fn call_files_refuse_a_content_type_that_is_not_text() {
    let request = payload(vec![(
        "image",
        object(vec![(
            FILE_KEY,
            object(vec![
                (BLOB_KEY, count(0)),
                (CONTENT_TYPE_KEY, Json::Bool(true)),
            ]),
        )]),
    )]);
    let mut blobs = vec![b"cat".to_vec()];

    let error = files_from_payload(&request, &mut blobs).unwrap_err();

    assert!(matches!(error, CallFilesError::InvalidMarker { key } if key == "image"));
}
