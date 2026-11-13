#![cfg(test)]

use std::sync::atomic::Ordering;

use std::time::Duration;

use rstest::rstest;

use crate::database_replica::availability::ReadTarget;
use crate::database_replica::client::DatabaseLifecycle as _;

use super::fixtures::fakes::{
    FakeError, Fixture, MINTED_URL, READER_URL, RecreateCall, RecreateStep, Step, WRITER_URL,
    cancel_after_started, secret, transport,
};

#[tokio::test]
async fn writer_then_reader_connect() {
    let fixture = Fixture::new();
    fixture
        .routing
        .connect(None)
        .await
        .expect("both sides connect");
    assert_eq!(fixture.events(), ["writer:connect", "reader:connect"]);
    assert_eq!(fixture.writer().timeouts(), [None]);
    assert_eq!(fixture.reader().timeouts(), [None]);
    assert!(
        !fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(
        !fixture
            .routing
            .availability
            .writer_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(fixture.informed("[writer] DB connected"));
    assert!(fixture.informed("[reader] DB connected"));
}

#[tokio::test]
async fn already_connected_logs_success() {
    let fixture = Fixture::new();
    fixture.writer().set_connected(true);
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    fixture
        .routing
        .connect(None)
        .await
        .expect("the reader connects");
    assert!(!fixture.has_event("writer:connect"));
    assert!(fixture.has_event("reader:connect"));
    assert!(
        !fixture
            .routing
            .availability
            .writer_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(fixture.informed("[writer] DB connected"));
}

#[tokio::test]
async fn connect_timeout_forwarded() {
    let fixture = Fixture::new();
    let timeout = Duration::from_secs(7);
    fixture
        .routing
        .connect(Some(timeout))
        .await
        .expect("both sides connect");
    assert_eq!(fixture.writer().timeouts(), [Some(timeout)]);
    assert_eq!(fixture.reader().timeouts(), [Some(timeout)]);
}

#[tokio::test]
async fn reader_failure_warns_and_falls_back() {
    let fixture = Fixture::new();
    fixture.reader().script_connect(Step::Fail);
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    fixture
        .routing
        .connect(None)
        .await
        .expect("a degraded reader is not fatal");
    assert!(
        fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(
        !fixture
            .routing
            .availability
            .writer_unavailable
            .load(Ordering::SeqCst)
    );
    assert_eq!(fixture.routing.read_target(), ReadTarget::Writer);
    assert!(fixture.warned(
        "Failed to connect to read replica DB: connect failed. Falling back to the writer for reads until the reader is reachable."
    ));
}

#[tokio::test]
async fn writer_failure_warns_and_serves_reader() {
    let fixture = Fixture::new();
    fixture.writer().script_connect(Step::Fail);
    fixture
        .routing
        .availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    fixture
        .routing
        .connect(None)
        .await
        .expect("a degraded writer is not fatal while the reader connects");
    assert!(
        fixture
            .routing
            .availability
            .writer_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(
        !fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
    assert_eq!(fixture.routing.read_target(), ReadTarget::Reader);
    assert!(fixture.warned(
        "Failed to connect to primary (writer) DB: connect failed. Serving reads from the read replica; writes will fail until the writer recovers."
    ));
}

#[tokio::test]
async fn both_fail_raise_original_writer_without_flag_rewrite() {
    let fixture = Fixture::new();
    fixture.writer().fail_connect_with(FakeError::Connect);
    fixture.reader().fail_connect_with(FakeError::Disconnect);
    let error = fixture
        .routing
        .connect(None)
        .await
        .expect_err("both sides failing is fatal");
    assert_eq!(error, FakeError::Connect);
    assert!(
        !fixture
            .routing
            .availability
            .writer_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(
        !fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn connect_cancel_stops_sequence() {
    let fixture = Fixture::new();
    fixture.writer().script_connect(Step::Blocked);
    cancel_after_started(
        fixture.routing.connect(None),
        fixture.writer().entered().notified(),
    )
    .await;
    assert!(!fixture.has_event("reader:connect"));
    assert!(
        !fixture
            .routing
            .availability
            .writer_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(
        !fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn writer_only_is_connected() {
    let fixture = Fixture::new();
    fixture.writer().set_connected(true);
    assert!(fixture.routing.is_connected());
    fixture.writer().set_connected(false);
    fixture.reader().set_connected(true);
    assert!(!fixture.routing.is_connected());
}

#[tokio::test]
async fn disconnect_attempts_both() {
    let fixture = Fixture::new();
    fixture.writer().script_disconnect(Step::Fail);
    let error = fixture
        .routing
        .disconnect(None)
        .await
        .expect_err("the writer failure surfaces");
    assert_eq!(error, FakeError::Disconnect);
    assert_eq!(fixture.events(), ["writer:disconnect", "reader:disconnect"]);
}

#[tokio::test]
async fn disconnect_logs_each_and_raises_first() {
    let fixture = Fixture::new();
    fixture.writer().fail_disconnect_with(FakeError::Disconnect);
    fixture.reader().fail_disconnect_with(FakeError::Refresh);
    let error = fixture
        .routing
        .disconnect(None)
        .await
        .expect_err("the first failure surfaces");
    assert_eq!(error, FakeError::Disconnect);
    assert!(fixture.captured.has(
        "WARN",
        "Error disconnecting Prisma client: disconnect failed"
    ));
    assert!(
        fixture
            .captured
            .has("WARN", "Error disconnecting Prisma client: refresh failed")
    );
}

#[tokio::test]
async fn disconnect_cancel_propagates() {
    let fixture = Fixture::new();
    fixture.writer().script_disconnect(Step::Blocked);
    cancel_after_started(
        fixture.routing.disconnect(None),
        fixture.writer().entered().notified(),
    )
    .await;
    assert!(!fixture.has_event("reader:disconnect"));
    assert!(
        !fixture
            .captured
            .has("WARN", "Error disconnecting Prisma client")
    );
}

#[tokio::test]
async fn refresh_start_stop_order() {
    let fixture = Fixture::new();
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    fixture
        .routing
        .availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    fixture
        .routing
        .start_refresh()
        .await
        .expect("refresh tasks start on both sides");
    fixture
        .routing
        .stop_refresh()
        .await
        .expect("refresh tasks stop on both sides");
    assert_eq!(
        fixture.events(),
        [
            "writer:start_refresh",
            "reader:start_refresh",
            "writer:stop_refresh",
            "reader:stop_refresh",
        ]
    );
}

#[tokio::test]
async fn refresh_failure_stops_sequence() {
    let fixture = Fixture::new();
    fixture.writer().script_start_refresh(Step::Fail);
    let error = fixture
        .routing
        .start_refresh()
        .await
        .expect_err("the writer start failure surfaces");
    assert_eq!(error, FakeError::Refresh);
    assert!(!fixture.has_event("reader:start_refresh"));
    fixture.writer().script_stop_refresh(Step::Fail);
    let stop_error = fixture
        .routing
        .stop_refresh()
        .await
        .expect_err("the writer stop failure surfaces");
    assert_eq!(stop_error, FakeError::Refresh);
    assert!(!fixture.has_event("reader:stop_refresh"));
}

#[tokio::test]
async fn stale_writer_generation_skips_reader() {
    let fixture = Fixture::new();
    fixture.writer().script_recreate(RecreateStep::Stale);
    let availability = &fixture.routing.availability;
    availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    let url = secret(WRITER_URL);
    let recreated = fixture
        .routing
        .recreate(&url, None, Some(4))
        .await
        .expect("a stale generation is not an error");
    assert!(!recreated);
    assert!(!fixture.has_event("reader:recreate"));
    assert!(availability.writer_unavailable.load(Ordering::SeqCst));
    assert!(availability.reader_unavailable.load(Ordering::SeqCst));
    assert_eq!(
        fixture.writer().recreates(),
        [RecreateCall {
            url: WRITER_URL.to_owned(),
            transport: None,
            generation: Some(4),
        }]
    );
}

#[tokio::test]
async fn writer_recreate_error_preserves_flags() {
    let fixture = Fixture::new();
    fixture.writer().script_recreate(RecreateStep::Fail);
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    fixture
        .routing
        .availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    let url = secret(WRITER_URL);
    let error = fixture
        .routing
        .recreate(&url, None, None)
        .await
        .expect_err("the writer failure surfaces");
    assert_eq!(error, FakeError::Recreate);
    assert!(
        fixture
            .routing
            .availability
            .writer_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(
        fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(!fixture.has_event("reader:recreate"));
}

#[tokio::test]
async fn token_reader_fresh_url() {
    let fixture = Fixture::new();
    fixture.reader().with_token_refresh("RDS IAM token");
    fixture.reader().with_minted(Ok(Some(secret(MINTED_URL))));
    let limits = transport(7);
    let url = secret(WRITER_URL);
    let recreated = fixture
        .routing
        .recreate(&url, Some(&limits), Some(2))
        .await
        .expect("both sides recreate");
    assert!(recreated);
    assert_eq!(
        fixture.writer().recreates(),
        [RecreateCall {
            url: WRITER_URL.to_owned(),
            transport: Some(7),
            generation: Some(2),
        }]
    );
    assert_eq!(
        fixture.reader().recreates(),
        [RecreateCall {
            url: MINTED_URL.to_owned(),
            transport: Some(7),
            generation: None,
        }]
    );
    assert!(
        !fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn falsey_token_reader_exact_error(#[case] minted: Option<&str>) {
    let fixture = Fixture::new();
    fixture.reader().with_token_refresh("RDS IAM token");
    fixture.reader().with_minted(Ok(minted.map(secret)));
    let url = secret(WRITER_URL);
    let recreated = fixture
        .routing
        .recreate(&url, None, None)
        .await
        .expect("the writer replacement still returns true");
    assert!(recreated);
    assert!(
        fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(!fixture.has_event("reader:recreate"));
    assert!(fixture.warned(
        "Failed to recreate reader Prisma client: Failed to generate fresh RDS IAM token for read replica. Reads will fall back to the writer until the reader recovers."
    ));
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn missing_password_reader_exact_error(#[case] reader_url: Option<&str>) {
    let fixture = Fixture::with_reader_url(reader_url);
    let url = secret(WRITER_URL);
    let recreated = fixture
        .routing
        .recreate(&url, None, None)
        .await
        .expect("the writer replacement still returns true");
    assert!(recreated);
    assert!(
        fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(!fixture.has_event("reader:recreate"));
    assert!(fixture.warned(
        "Failed to recreate reader Prisma client: DATABASE_URL_READ_REPLICA not set; cannot recreate read replica client. Reads will fall back to the writer until the reader recovers."
    ));
}

#[tokio::test]
async fn reader_recreate_error_after_writer_success() {
    let fixture = Fixture::new();
    fixture.reader().script_recreate(RecreateStep::Fail);
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    let url = secret(WRITER_URL);
    let recreated = fixture
        .routing
        .recreate(&url, None, None)
        .await
        .expect("the writer replacement still returns true");
    assert!(recreated);
    assert!(
        !fixture
            .routing
            .availability
            .writer_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(
        fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(fixture.has_event("writer:recreate"));
    assert!(fixture.has_event("reader:recreate"));
    assert!(fixture.warned(
        "Failed to recreate reader Prisma client: the read replica client could not be recreated: recreate failed. Reads will fall back to the writer until the reader recovers."
    ));
}

#[tokio::test]
async fn reader_recreate_cancel_not_swallowed() {
    let fixture = Fixture::new();
    fixture.reader().script_recreate(RecreateStep::Blocked);
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    let url = secret(WRITER_URL);
    cancel_after_started(
        fixture.routing.recreate(&url, None, None),
        fixture.reader().entered().notified(),
    )
    .await;
    assert!(fixture.has_event("writer:recreate"));
    assert!(
        !fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(
        !fixture
            .routing
            .availability
            .writer_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(
        !fixture
            .captured
            .has("WARN", "Failed to recreate reader Prisma client")
    );
}

#[tokio::test]
async fn token_reader_mint_error_degrades() {
    let fixture = Fixture::new();
    fixture.reader().with_token_refresh("RDS IAM token");
    fixture.reader().with_minted(Err(FakeError::Mint));
    let url = secret(WRITER_URL);
    let recreated = fixture
        .routing
        .recreate(&url, None, None)
        .await
        .expect("the writer replacement still returns true");
    assert!(recreated);
    assert!(
        fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(!fixture.has_event("reader:recreate"));
    assert!(fixture.warned(
        "Failed to recreate reader Prisma client: the read replica credential could not be minted: mint failed. Reads will fall back to the writer until the reader recovers."
    ));
}

#[tokio::test]
async fn reader_recovery_clears_flag() {
    let fixture = Fixture::new();
    fixture
        .routing
        .availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    let url = secret(WRITER_URL);
    let recreated = fixture
        .routing
        .recreate(&url, None, None)
        .await
        .expect("both sides recreate");
    assert!(recreated);
    assert!(
        !fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
    assert_eq!(
        fixture.reader().recreates(),
        [RecreateCall {
            url: READER_URL.to_owned(),
            transport: None,
            generation: None,
        }]
    );
}
