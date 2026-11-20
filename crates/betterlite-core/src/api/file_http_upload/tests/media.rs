use std::time::Duration;

use http::Method;
use http::header::CONTENT_TYPE;
use rstest::rstest;

use crate::api::file_http_upload::error::UploadError;
use crate::api::file_http_upload::media::MEDIA_UPLOAD_BLOCK_SIZE;
use crate::api::file_http_upload::plan::{MediaPlan, UploadPlan};
use crate::api::file_http_upload::service::Mode;

use super::{API_BASE, Harness, answer, create_call, params};

pub(super) fn media_plan(pieces: Vec<Vec<u8>>, content_type: Option<String>) -> UploadPlan {
    UploadPlan::Media(Box::new(MediaPlan {
        body_stream: Box::new(pieces.into_iter()),
        content_type,
    }))
}

#[tokio::test]
async fn media_upload_blocks() {
    let mut full = vec![b'a'; MEDIA_UPLOAD_BLOCK_SIZE];
    full.extend_from_slice(b"tail");
    let mut harness = Harness::new(
        media_plan(
            vec![b"head".to_vec(), full, b"".to_vec()],
            Some(String::from("application/jsonl")),
        ),
        API_BASE,
        vec![answer(201, &[], b"{}")],
    );
    let mut call = create_call();
    call.timeout = Some(Duration::from_secs(30));
    harness
        .run(Mode::Asynchronous, &call, &params())
        .await
        .unwrap();
    let seen = harness.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, Method::POST);
    assert_eq!(
        seen[0].headers.get(CONTENT_TYPE).expect("the media type"),
        "application/jsonl"
    );
    assert_eq!(seen[0].timeout, Some(Duration::from_secs(30)));
    assert_eq!(seen[0].body.len(), 2);
    // LiteLLM's _MEDIA_UPLOAD_BLOCK_SIZE is four MiB, exactly
    assert_eq!(MEDIA_UPLOAD_BLOCK_SIZE, 4_194_304);
    // the first block starts with head and fills to the limit, so the last eight bytes are the tail
    assert_eq!(seen[0].body[0].len(), MEDIA_UPLOAD_BLOCK_SIZE);
    assert!(seen[0].body[0].starts_with(b"head"));
    assert_eq!(seen[0].body[1], b"aaaatail".to_vec());
}

// an unset or empty configured type takes the octet-stream default, and an empty stream sends nothing
#[rstest]
#[case::unset(None)]
#[case::empty(Some(String::new()))]
#[tokio::test]
async fn media_upload_default_content_type(#[case] content_type: Option<String>) {
    let mut harness = Harness::new(
        media_plan(Vec::new(), content_type),
        API_BASE,
        vec![answer(200, &[], b"{}")],
    );
    harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap();
    let seen = harness.seen();
    assert!(seen[0].body.is_empty());
    assert_eq!(
        seen[0].headers.get(CONTENT_TYPE).expect("the media type"),
        "application/octet-stream"
    );
    assert_eq!(seen[0].timeout, None);
}

#[rstest]
#[case::accepted(202_u16, "media upload: unexpected status 202")]
#[case::no_content(204, "media upload: unexpected status 204")]
#[tokio::test]
async fn media_upload_unexpected_status(#[case] code: u16, #[case] expected: &str) {
    let mut harness = Harness::new(
        media_plan(vec![b"body".to_vec()], None),
        API_BASE,
        vec![answer(code, &[], b"")],
    );
    let error = harness
        .run(Mode::Sync, &create_call(), &params())
        .await
        .unwrap_err();
    match error {
        UploadError::Provider(provider) => {
            assert_eq!(provider.status_code, 500);
            assert!(provider.status_code_is_synthesized);
            assert_eq!(provider.message, expected);
        }
        other => panic!("expected a provider error, found {other}"),
    }
}

#[tokio::test]
async fn media_upload_status_error() {
    let mut harness = Harness::new(
        media_plan(vec![b"body".to_vec()], None),
        API_BASE,
        vec![answer(400, &[("content-type", "text/plain")], b"bad media")],
    );
    let error = harness
        .run(Mode::Asynchronous, &create_call(), &params())
        .await
        .unwrap_err();
    match error {
        UploadError::Provider(provider) => {
            assert_eq!(provider.status_code, 400);
            assert!(!provider.status_code_is_synthesized);
            assert_eq!(provider.message, "bad media");
        }
        other => panic!("expected a provider error, found {other}"),
    }
}
