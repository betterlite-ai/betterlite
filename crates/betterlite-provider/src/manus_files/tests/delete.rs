use betterlite_openai::files::records::{FileDeleted, FileKind};
use rstest::rstest;

use crate::api::file_contracts::config::{BaseFilesConfig as _, FileLogging};
use crate::manus_files::error::ManusFilesError;
use crate::manus_files::reply::decode_deleted_file;

use super::support::{NOW, params, plain_config, response};

#[test]
fn the_delete_reply_is_the_openai_file_deleted_record() {
    let deleted =
        decode_deleted_file(br#"{"id":"file-1","object":"file","deleted":true,"extra":1}"#)
            .expect("the reply is a file-deleted record");
    assert_eq!(
        deleted,
        FileDeleted {
            id: String::from("file-1"),
            object: FileKind::File,
            deleted: true,
        }
    );
}

#[rstest]
#[case::missing_deleted(r#"{"id":"file-1","object":"file"}"#)]
#[case::missing_id(r#"{"object":"file","deleted":true}"#)]
#[case::null_id(r#"{"id":null,"object":"file","deleted":true}"#)]
#[case::wrong_object(r#"{"id":"file-1","object":"list","deleted":true}"#)]
#[case::wrong_deleted(r#"{"id":"file-1","object":"file","deleted":"yes"}"#)]
#[case::not_json("not json")]
fn a_bad_delete_reply_is_refused(#[case] body: &str) {
    let error = decode_deleted_file(body.as_bytes()).expect_err("the reply is not a deleted file");
    assert!(matches!(error, ManusFilesError::DeleteResponse { .. }));
}

#[test]
fn the_trait_delete_response_returns_the_record() {
    let deleted = plain_config(NOW)
        .transform_delete_file_response(
            &response(r#"{"id":"file-2","object":"file","deleted":true}"#),
            &mut FileLogging::default(),
            &params("{}"),
        )
        .expect("the reply is a file-deleted record");
    assert_eq!(deleted.id, "file-2");
    assert!(deleted.deleted);
}
