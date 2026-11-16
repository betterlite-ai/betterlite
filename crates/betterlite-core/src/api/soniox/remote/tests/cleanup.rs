use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::AUTHORIZATION;

use super::support::{
    Clock, Outcome, RecordedBody, Script, TIMEOUT, auth, options, prepared, remote,
};
use crate::api::soniox::prepare::Prepared;
use crate::api::soniox::remote::cleanup::clean_up;

fn ok() -> Outcome {
    Outcome::Reply {
        status: 204,
        headers: Vec::new(),
        body: "",
    }
}

fn urls(transport: &Script) -> Vec<String> {
    transport
        .calls()
        .iter()
        .map(|call| call.url.clone())
        .collect()
}

async fn cleanup_with(
    selection: &[&str],
    transcription_id: Option<&str>,
    file_id: Option<&str>,
) -> Vec<String> {
    let mut chosen = options();
    chosen.cleanup = selection.iter().map(|name| (*name).to_owned()).collect();
    let transport = Script::new(vec![ok(), ok()]);
    let sleeper = Clock::default();
    let ready = prepared(chosen, &auth());
    clean_up(
        &remote(&transport, &sleeper, &ready),
        transcription_id,
        file_id,
    )
    .await;
    urls(&transport)
}

#[tokio::test]
async fn cleanup_does_nothing_for_an_empty_selection() {
    assert!(
        cleanup_with(&[], Some("job-1"), Some("file-1"))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn cleanup_deletes_the_transcription_before_the_file() {
    let mut chosen = options();
    chosen.cleanup = vec!["file".to_owned(), "transcription".to_owned()];
    let transport = Script::new(vec![ok(), ok()]);
    let sleeper = Clock::default();
    let ready = prepared(chosen, &auth());
    clean_up(
        &remote(&transport, &sleeper, &ready),
        Some("job-1"),
        Some("file-1"),
    )
    .await;
    let calls = transport.calls();
    assert_eq!(
        urls(&transport),
        vec![
            "https://api.soniox.test/v1/transcriptions/job-1".to_owned(),
            "https://api.soniox.test/v1/files/file-1".to_owned(),
        ]
    );
    assert_eq!(calls[0].method, Method::DELETE);
    assert_eq!(calls[0].headers.len(), 2);
    assert_eq!(
        calls[0]
            .headers
            .get(AUTHORIZATION)
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer sk-local"
    );
    assert_eq!(
        calls[0].headers.get("x-client").unwrap().to_str().unwrap(),
        "suite"
    );
    assert!(matches!(&calls[0].body, RecordedBody::Empty));
    assert_eq!(calls[0].deadline, TIMEOUT);
}

#[tokio::test]
async fn cleanup_deletes_only_the_selected_transcription() {
    assert_eq!(
        cleanup_with(&["transcription"], Some("job-1"), Some("file-1")).await,
        vec!["https://api.soniox.test/v1/transcriptions/job-1".to_owned()]
    );
}

#[tokio::test]
async fn cleanup_deletes_only_the_selected_file() {
    assert_eq!(
        cleanup_with(&["file"], Some("job-1"), Some("file-1")).await,
        vec!["https://api.soniox.test/v1/files/file-1".to_owned()]
    );
}

#[tokio::test]
async fn cleanup_matches_selection_names_exactly() {
    assert!(
        cleanup_with(&["File", "transcriptions"], Some("job-1"), Some("file-1"))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn cleanup_skips_empty_and_missing_ids() {
    assert!(
        cleanup_with(&["transcription", "file"], Some(""), Some(""))
            .await
            .is_empty()
    );
    assert!(
        cleanup_with(&["transcription", "file"], None, None)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn cleanup_attempts_the_file_after_the_transcription_fails() {
    let transport = Script::new(vec![Outcome::Fail(TransportError::Closed), ok()]);
    let sleeper = Clock::default();
    let ready = prepared(options(), &auth());
    clean_up(
        &remote(&transport, &sleeper, &ready),
        Some("job-1"),
        Some("file-1"),
    )
    .await;
    assert_eq!(
        urls(&transport),
        vec![
            "https://api.soniox.test/v1/transcriptions/job-1".to_owned(),
            "https://api.soniox.test/v1/files/file-1".to_owned(),
        ]
    );
}

#[tokio::test]
async fn cleanup_ignores_the_delete_status() {
    let transport = Script::new(vec![
        Outcome::Reply {
            status: 500,
            headers: Vec::new(),
            body: r#"{"error_message":"stale"}"#,
        },
        ok(),
    ]);
    let sleeper = Clock::default();
    let ready = prepared(options(), &auth());
    clean_up(
        &remote(&transport, &sleeper, &ready),
        Some("job-1"),
        Some("file-1"),
    )
    .await;
    assert_eq!(transport.calls().len(), 2);
}

#[tokio::test]
async fn cleanup_skips_a_delete_it_cannot_build() {
    let transport = Script::new(Vec::new());
    let sleeper = Clock::default();
    let bad_base = Prepared {
        api_base: "not a url".to_owned(),
        ..prepared(options(), &auth())
    };
    let bad_header = prepared(options(), &[("bad name", "value")]);
    clean_up(
        &remote(&transport, &sleeper, &bad_base),
        Some("job-1"),
        Some("file-1"),
    )
    .await;
    clean_up(
        &remote(&transport, &sleeper, &bad_header),
        Some("job-1"),
        Some("file-1"),
    )
    .await;
    assert!(transport.calls().is_empty());
}

fn sent_credentials(transport: &Script) -> Vec<(String, String)> {
    transport
        .calls()
        .iter()
        .map(|call| {
            let bearer = call.headers.get(AUTHORIZATION).unwrap().to_str().unwrap();
            (call.url.clone(), bearer.to_owned())
        })
        .collect()
}

#[tokio::test]
async fn cleanup_sends_the_request_local_credential() {
    let transport = Script::new(vec![ok(), ok(), ok(), ok()]);
    let sleeper = Clock::default();
    let other = prepared(options(), &[("Authorization", "Bearer sk-neighbour")]);
    let ready = prepared(options(), &[("Authorization", "Bearer sk-local")]);
    clean_up(
        &remote(&transport, &sleeper, &other),
        Some("job-n"),
        Some("file-n"),
    )
    .await;
    clean_up(
        &remote(&transport, &sleeper, &ready),
        Some("job-1"),
        Some("file-1"),
    )
    .await;
    let base = "https://api.soniox.test/v1";
    assert_eq!(
        sent_credentials(&transport),
        vec![
            (
                format!("{base}/transcriptions/job-n"),
                "Bearer sk-neighbour".to_owned(),
            ),
            (
                format!("{base}/files/file-n"),
                "Bearer sk-neighbour".to_owned(),
            ),
            (
                format!("{base}/transcriptions/job-1"),
                "Bearer sk-local".to_owned(),
            ),
            (format!("{base}/files/file-1"), "Bearer sk-local".to_owned(),),
        ]
    );
}
