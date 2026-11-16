use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::AUTHORIZATION;

use super::support::{
    Clock, Outcome, RecordedBody, Script, TIMEOUT, auth, options, prepared, remote,
};

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
    remote(&transport, &sleeper, &ready)
        .cleanup(transcription_id, file_id)
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
    remote(&transport, &sleeper, &ready)
        .cleanup(Some("job-1"), Some("file-1"))
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
    remote(&transport, &sleeper, &ready)
        .cleanup(Some("job-1"), Some("file-1"))
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
    remote(&transport, &sleeper, &ready)
        .cleanup(Some("job-1"), Some("file-1"))
        .await;
    assert_eq!(transport.calls().len(), 2);
}

#[tokio::test]
async fn cleanup_sends_the_request_local_credential() {
    let transport = Script::new(vec![ok(), ok(), ok(), ok()]);
    let sleeper = Clock::default();
    let other = prepared(options(), &[("Authorization", "Bearer sk-neighbour")]);
    let ready = prepared(options(), &[("Authorization", "Bearer sk-local")]);
    remote(&transport, &sleeper, &other)
        .cleanup(Some("job-n"), Some("file-n"))
        .await;
    remote(&transport, &sleeper, &ready)
        .cleanup(Some("job-1"), Some("file-1"))
        .await;
    let sent: Vec<(String, String)> = transport
        .calls()
        .iter()
        .map(|call| {
            (
                call.url.clone(),
                call.headers
                    .get(AUTHORIZATION)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
            )
        })
        .collect();
    let expected = [
        ("transcriptions/job-n", "Bearer sk-neighbour"),
        ("files/file-n", "Bearer sk-neighbour"),
        ("transcriptions/job-1", "Bearer sk-local"),
        ("files/file-1", "Bearer sk-local"),
    ];
    assert_eq!(sent.len(), expected.len());
    for ((url, credential), (path, bearer)) in sent.iter().zip(expected) {
        assert_eq!(url, &format!("https://api.soniox.test/v1/{path}"));
        assert_eq!(credential, bearer);
    }
}
