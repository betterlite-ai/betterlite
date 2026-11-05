#![cfg(test)]

use super::shared::{DONE_ALLOW, UPLOADED, body_text, sanitizer, vendor};
use crate::prompt_security_files::wire::Action;

#[tokio::test]
async fn upload_poll_sends_multipart_file_with_app_id_and_alias() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let result = sanitizer(&server.base)
        .sanitize_file_content(b"hello", "image.png", Some("alias-1"))
        .await
        .expect("sanitized");
    assert_eq!(result.action, Action::Allow);
    let uploads = server.requests("POST");
    let upload = uploads.first().expect("upload");
    assert_eq!(upload.headers["app-id"], "vendor-key");
    assert_eq!(upload.headers["x-litellm-key-alias"], "alias-1");
    let body = body_text(upload);
    assert!(body.contains(r#"name="file"; filename="image.png""#));
    assert!(body.contains("Content-Type: image/png"));
    assert!(body.contains("hello"));
}

#[tokio::test]
async fn upload_poll_polls_with_job_id_and_the_same_headers() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    sanitizer(&server.base)
        .sanitize_file_content(b"hello", "document.pdf", Some("alias-1"))
        .await
        .expect("sanitized");
    let polls = server.requests("GET");
    let poll = polls.first().expect("poll");
    assert_eq!(poll.query, "jobId=job-1");
    assert_eq!(poll.headers["app-id"], "vendor-key");
    assert_eq!(poll.headers["x-litellm-key-alias"], "alias-1");
}

#[tokio::test]
async fn upload_poll_omits_the_alias_header_when_absent_or_empty() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let guardrail = sanitizer(&server.base);
    guardrail
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect("none");
    guardrail
        .sanitize_file_content(b"a", "image.png", Some(""))
        .await
        .expect("empty");
    let uploads = server.requests("POST");
    assert_eq!(uploads.len(), 2);
    assert!(
        uploads
            .iter()
            .all(|upload| !upload.headers.contains_key("x-litellm-key-alias"))
    );
}
