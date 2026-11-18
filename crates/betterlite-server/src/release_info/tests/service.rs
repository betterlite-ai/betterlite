use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::Method;
use rstest::rstest;

use super::support::{Answer, RELEASE_URL, Script, advance, fake_clock};
use crate::release_info::records::LatestRelease;
use crate::release_info::service::{ReleaseError, ReleaseService};

const RELEASE_RECORD: &str = r#"{"tag_name":"v2.4.6","html_url":"https://example.test/release","body":"* feat: a\n* fix: b\n* chore: c\n"}"#;

struct Fixture {
    service: ReleaseService<Script>,
    script: Script,
    now: Arc<Mutex<f64>>,
}

fn fixture(answers: Vec<Answer>) -> Fixture {
    let script = Script::new(answers);
    let (clock, now) = fake_clock(1_000.0);
    Fixture {
        service: ReleaseService::new(script.clone(), RELEASE_URL.to_owned(), clock),
        script,
        now,
    }
}

fn success() -> Answer {
    Answer::Reply {
        status: 200,
        body: RELEASE_RECORD.as_bytes().to_vec(),
    }
}

fn failure(status: u16) -> Answer {
    Answer::Reply {
        status,
        body: Vec::new(),
    }
}

#[tokio::test]
async fn fetches_the_release_from_the_configured_source() {
    let fixture = fixture(vec![success()]);
    let release = fixture
        .service
        .get()
        .await
        .expect("the release should be served");
    let LatestRelease::Available(info) = release else {
        panic!("a valid release record should be available");
    };
    assert_eq!(info.version, "2.4.6");
    assert_eq!(info.new_features, 1);
    assert_eq!(info.bug_fixes, 1);
    assert_eq!(info.other_updates, 1);
    assert_eq!(info.release_url, "https://example.test/release");
    assert_eq!(fixture.script.calls(), 1);
    let seen = fixture.script.seen();
    let request = seen.first().expect("the request should be recorded");
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url, RELEASE_URL);
    assert_eq!(request.deadline, Duration::from_secs(5));
}

#[rstest]
#[case::inside_the_hour(3_599.0, 1)]
#[case::exactly_at_the_hour(3_600.0, 1)]
#[case::past_the_hour(3_600.5, 2)]
#[tokio::test]
async fn keeps_a_success_for_an_hour(#[case] elapsed: f64, #[case] expected_calls: usize) {
    let fixture = fixture(vec![success(), success()]);
    let first = fixture
        .service
        .get()
        .await
        .expect("the first read is served");
    assert!(matches!(first, LatestRelease::Available(_)));
    advance(&fixture.now, elapsed);
    let second = fixture
        .service
        .get()
        .await
        .expect("the second read is served");
    assert!(matches!(second, LatestRelease::Available(_)));
    assert_eq!(fixture.script.calls(), expected_calls);
}

#[rstest]
#[case::inside_five_minutes(299.0, 1)]
#[case::exactly_at_five_minutes(300.0, 1)]
#[case::past_five_minutes(300.5, 2)]
#[tokio::test]
async fn keeps_an_unavailable_answer_for_five_minutes(
    #[case] elapsed: f64,
    #[case] expected_calls: usize,
) {
    let fixture = fixture(vec![failure(500), failure(500)]);
    let first = fixture
        .service
        .get()
        .await
        .expect("the first read is an answer");
    assert!(matches!(first, LatestRelease::Unavailable(_)));
    advance(&fixture.now, elapsed);
    let second = fixture
        .service
        .get()
        .await
        .expect("the second read is an answer");
    assert!(matches!(second, LatestRelease::Unavailable(_)));
    assert_eq!(fixture.script.calls(), expected_calls);
}

#[tokio::test]
async fn concurrent_misses_share_a_single_fetch() {
    let fixture = fixture(vec![success(), success()]);
    let (first, second) = tokio::join!(fixture.service.get(), fixture.service.get());
    let first = first.expect("the first read is served");
    let second = second.expect("the second read is served");
    assert_eq!(first, second);
    assert!(matches!(first, LatestRelease::Available(_)));
    assert_eq!(fixture.script.calls(), 1);
}

#[tokio::test]
async fn a_cancelled_read_is_not_cached() {
    let fixture = fixture(vec![success(), success()]);
    let cancelled = tokio::time::timeout(Duration::ZERO, fixture.service.get()).await;
    assert!(cancelled.is_err());
    let served = fixture
        .service
        .get()
        .await
        .expect("the next read should be served");
    assert!(matches!(served, LatestRelease::Available(_)));
    assert_eq!(fixture.script.calls(), 2);
}

#[tokio::test]
async fn a_transport_failure_becomes_unavailable() {
    let fixture = fixture(vec![Answer::Closed]);
    let release = fixture
        .service
        .get()
        .await
        .expect("a transport failure is an answer");
    let LatestRelease::Unavailable(unavailable) = release else {
        panic!("a transport failure should be unavailable");
    };
    assert!(!unavailable.reason.is_empty());
    assert_eq!(fixture.script.calls(), 1);
}

#[tokio::test]
async fn an_invalid_payload_becomes_unavailable() {
    let fixture = fixture(vec![Answer::Reply {
        status: 200,
        body: b"not a release record".to_vec(),
    }]);
    let release = fixture
        .service
        .get()
        .await
        .expect("an invalid payload is an answer");
    let LatestRelease::Unavailable(unavailable) = release else {
        panic!("an invalid payload should be unavailable");
    };
    assert!(
        unavailable
            .reason
            .starts_with("GitHub release payload was not the expected shape: ")
    );
}

#[tokio::test]
async fn an_unusable_source_is_an_error() {
    let script = Script::new(Vec::new());
    let (clock, _now) = fake_clock(1_000.0);
    let service = ReleaseService::new(script.clone(), "not a url".to_owned(), clock);
    let outcome = service.get().await;
    assert!(matches!(outcome, Err(ReleaseError::Source(_))));
    assert_eq!(script.calls(), 0);
}
