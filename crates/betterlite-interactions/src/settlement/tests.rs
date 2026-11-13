#![cfg(test)]

use std::collections::VecDeque;
use std::future::ready;
use std::io::{Result as IoResult, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::{bill_settled_interaction, release_open_reservation, settle_before_delete};
use crate::polling::context::{InteractionPollContext, SettlementFlag};
use crate::polling::host::{FetchFuture, FetchInteraction, SettlementFuture, SettlementHost};
use crate::polling::registry::BackgroundPollRegistry;
use crate::types::response::InteractionsApiResponse;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
struct FakeError(&'static str);

#[derive(Default)]
struct Recordings {
    billed: Mutex<Vec<String>>,
    released: Mutex<Vec<String>>,
}

#[derive(Clone)]
struct FakeHost {
    recordings: Arc<Recordings>,
    reservation: Option<String>,
    billing_fails: bool,
    release_fails: bool,
}

impl FakeHost {
    fn billed(&self) -> Vec<String> {
        self.recordings
            .billed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn released(&self) -> Vec<String> {
        self.recordings
            .released
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SettlementHost for FakeHost {
    type Reservation = String;
    type BillingError = FakeError;
    type ReleaseError = FakeError;

    fn budget_reservation(&self) -> Option<String> {
        self.reservation.clone()
    }

    fn log_background_completion<'host>(
        &'host self,
        response: &'host InteractionsApiResponse,
    ) -> SettlementFuture<'host, Result<(), FakeError>> {
        let recordings = Arc::clone(&self.recordings);
        let interaction_id = response.id.clone().unwrap_or_default();
        let fails = self.billing_fails;
        Box::pin(async move {
            recordings
                .billed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(interaction_id);
            if fails {
                return Err(FakeError("billing failed"));
            }
            Ok(())
        })
    }

    fn release_reservation<'host>(
        &'host self,
        reservation: &'host String,
    ) -> SettlementFuture<'host, Result<(), FakeError>> {
        self.recordings
            .released
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(reservation.clone());
        if self.release_fails {
            return Box::pin(ready(Err(FakeError("release failed"))));
        }
        Box::pin(ready(Ok(())))
    }
}

fn host(reservation: Option<&str>) -> FakeHost {
    FakeHost {
        recordings: Arc::default(),
        reservation: reservation.map(str::to_owned),
        billing_fails: false,
        release_fails: false,
    }
}

#[derive(Clone, Default)]
struct Script {
    answers: Arc<Mutex<VecDeque<Result<InteractionsApiResponse, FakeError>>>>,
    calls: Arc<AtomicUsize>,
}

impl Script {
    fn returning(answers: Vec<Result<InteractionsApiResponse, FakeError>>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(VecDeque::from(answers))),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn fetch(&self) -> FetchInteraction<FakeHost, FakeError> {
        let answers = Arc::clone(&self.answers);
        let calls = Arc::clone(&self.calls);
        Arc::new(move |_context| -> FetchFuture<FakeError> {
            calls.fetch_add(1, Ordering::SeqCst);
            let answer = answers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or_else(|| Err(FakeError("no scripted answer")));
            Box::pin(async move { answer })
        })
    }
}

fn answer(status: &str, with_usage: bool) -> InteractionsApiResponse {
    let usage = if with_usage {
        r#","usage":{"total_tokens":5}"#
    } else {
        ""
    };
    let text = format!(r#"{{"id":"interaction-1","status":"{status}"{usage}}}"#);
    serde_json::from_str(&text).expect("the scripted response deserializes")
}

fn poll_context(host: FakeHost) -> InteractionPollContext<FakeHost> {
    InteractionPollContext {
        interaction_id: "interaction-1".to_owned(),
        custom_llm_provider: "gemini".to_owned(),
        host,
        api_key: None,
        api_base: None,
        initial_interval: Duration::from_secs(3600),
        max_interval: Duration::from_secs(3600),
        timeout: Duration::from_secs(3600),
        settlement: SettlementFlag::default(),
    }
}

// an hour-long first interval keeps the spawned poll asleep for the whole test
fn registry_with(host: FakeHost) -> BackgroundPollRegistry<FakeHost> {
    let mut registry = BackgroundPollRegistry::default();
    let poll_fetch: FetchInteraction<FakeHost, FakeError> =
        Arc::new(|_context| Box::pin(ready(Ok(answer("in_progress", false)))));
    assert!(registry.schedule(poll_context(host), poll_fetch));
    registry
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("captured logs are UTF-8")
    }
}

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}

#[tokio::test]
async fn no_active_poll_returns_before_fetching_or_claiming() {
    let registry = BackgroundPollRegistry::default();
    let script = Script::default();
    settle_before_delete(&registry, "interaction-1", script.fetch())
        .await
        .expect("no active poll settles nothing");
    assert_eq!(script.calls(), 0);
}

#[rstest::rstest]
#[case::completed("completed")]
#[case::requires_action("requires_action")]
#[case::failed("failed")]
#[case::cancelled("cancelled")]
#[case::incomplete("incomplete")]
#[case::budget_exceeded("budget_exceeded")]
#[tokio::test]
async fn bills_terminal_with_usage(#[case] status: &str) {
    let host = host(Some("reservation-1"));
    let registry = registry_with(host.clone());
    let script = Script::returning(vec![Ok(answer(status, true))]);
    settle_before_delete(&registry, "interaction-1", script.fetch())
        .await
        .expect("billing succeeds");
    assert_eq!(script.calls(), 1);
    assert_eq!(host.billed(), vec!["interaction-1".to_owned()]);
    assert!(host.released().is_empty());
}

#[rstest::rstest]
#[case::completed("completed")]
#[case::requires_action("requires_action")]
#[case::failed("failed")]
#[tokio::test]
async fn releases_without_usage(#[case] status: &str) {
    let host = host(Some("reservation-1"));
    let registry = registry_with(host.clone());
    let script = Script::returning(vec![Ok(answer(status, false))]);
    settle_before_delete(&registry, "interaction-1", script.fetch())
        .await
        .expect("a terminal response without usage releases");
    assert!(host.billed().is_empty());
    assert_eq!(host.released(), vec!["reservation-1".to_owned()]);
}

#[rstest::rstest]
#[case::in_progress("in_progress")]
#[case::queued("queued")]
#[case::unknown("mystery")]
#[tokio::test]
async fn releases_non_terminal(#[case] status: &str) {
    let host = host(Some("reservation-1"));
    let registry = registry_with(host.clone());
    let script = Script::returning(vec![Ok(answer(status, true))]);
    settle_before_delete(&registry, "interaction-1", script.fetch())
        .await
        .expect("a queued or running interaction releases");
    assert!(host.billed().is_empty());
    assert_eq!(host.released(), vec!["reservation-1".to_owned()]);
}

#[tokio::test]
async fn fetch_error_releases_the_reservation() {
    let host = host(Some("reservation-1"));
    let registry = registry_with(host.clone());
    let context = registry
        .active_poll("interaction-1")
        .expect("the poll is active");
    let script = Script::returning(vec![Err(FakeError("provider is down"))]);
    settle_before_delete(&registry, "interaction-1", script.fetch())
        .await
        .expect("an unfetchable interaction settles by releasing");
    assert_eq!(script.calls(), 1);
    assert!(host.billed().is_empty());
    assert_eq!(host.released(), vec!["reservation-1".to_owned()]);
    assert!(!context.settlement.claim_settlement());
}

#[tokio::test]
async fn fetch_error_claim_lost_releases_nothing() {
    let host = host(Some("reservation-1"));
    let registry = registry_with(host.clone());
    let context = registry
        .active_poll("interaction-1")
        .expect("the poll is active");
    assert!(context.settlement.claim_settlement());
    let script = Script::returning(vec![Err(FakeError("provider is down"))]);
    settle_before_delete(&registry, "interaction-1", script.fetch())
        .await
        .expect("the poll task owns the settlement");
    assert!(host.billed().is_empty());
    assert!(host.released().is_empty());
}

#[tokio::test]
async fn settles_with_the_create_credentials_from_the_poll_context() {
    let host = host(Some("reservation-1"));
    let mut registry = BackgroundPollRegistry::default();
    let mut context = poll_context(host);
    context.custom_llm_provider = "vertex_ai".to_owned();
    context.api_key = Some(SecretString::from("create-key"));
    context.api_base = Some("https://example.test".to_owned());
    let fetched = Arc::new(Mutex::new(None));
    let recorder = Arc::clone(&fetched);
    let fetch: FetchInteraction<FakeHost, FakeError> = Arc::new(move |context| {
        *recorder.lock().unwrap_or_else(PoisonError::into_inner) = Some(context);
        Box::pin(ready(Ok(answer("completed", true))))
    });
    assert!(registry.schedule(context, Arc::clone(&fetch)));
    settle_before_delete(&registry, "interaction-1", fetch)
        .await
        .expect("the fetched interaction bills");
    let seen = fetched
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .expect("the delete-time fetch runs");
    assert_eq!(seen.interaction_id, "interaction-1");
    assert_eq!(seen.custom_llm_provider, "vertex_ai");
    assert_eq!(seen.api_base.as_deref(), Some("https://example.test"));
    let api_key = seen
        .api_key
        .expect("the create's key travels with the poll");
    assert_eq!(api_key.expose_secret(), "create-key");
}

#[tokio::test]
async fn claim_lost_does_not_bill_or_release() {
    let host = host(Some("reservation-1"));
    let registry = registry_with(host.clone());
    let context = registry
        .active_poll("interaction-1")
        .expect("the poll is active");
    assert!(context.settlement.claim_settlement());
    let script = Script::returning(vec![Ok(answer("completed", true))]);
    settle_before_delete(&registry, "interaction-1", script.fetch())
        .await
        .expect("a lost claim returns quietly");
    assert_eq!(script.calls(), 1);
    assert!(host.billed().is_empty());
    assert!(host.released().is_empty());
}

#[tokio::test]
async fn bill_error_releases_then_fails() {
    let mut host = host(Some("reservation-1"));
    host.billing_fails = true;
    let response = answer("completed", true);
    let error = bill_settled_interaction(&host, &response)
        .await
        .expect_err("the billing error reaches the caller");
    assert_eq!(error, FakeError("billing failed"));
    assert_eq!(host.released(), vec!["reservation-1".to_owned()]);
}

#[tokio::test]
async fn settle_bill_error_releases_then_fails() {
    let mut host = host(Some("reservation-1"));
    host.billing_fails = true;
    let registry = registry_with(host.clone());
    let script = Script::returning(vec![Ok(answer("completed", true))]);
    let error = settle_before_delete(&registry, "interaction-1", script.fetch())
        .await
        .expect_err("the billing error reaches the delete");
    assert_eq!(error, FakeError("billing failed"));
    assert_eq!(host.released(), vec!["reservation-1".to_owned()]);
}

#[tokio::test]
async fn release_error_is_swallowed() {
    let (logs, _guard) = capture_logs();
    let mut host = host(Some("reservation-1"));
    host.release_fails = true;
    release_open_reservation(&host).await;
    assert_eq!(host.released(), vec!["reservation-1".to_owned()]);
    assert!(
        logs.contents().contains(
            "Failed to release budget reservation for an unbilled background interaction"
        ),
        "the release failure is logged: {}",
        logs.contents()
    );
}

#[tokio::test]
async fn no_reservation_is_noop() {
    let host = host(None);
    release_open_reservation(&host).await;
    assert!(host.released().is_empty());
    let response = answer("completed", true);
    bill_settled_interaction(&host, &response)
        .await
        .expect("billing still runs without a reservation");
    assert_eq!(host.billed(), vec!["interaction-1".to_owned()]);
    assert!(host.released().is_empty());
}
