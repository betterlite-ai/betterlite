#![cfg(test)]

use std::collections::VecDeque;
use std::future::ready;
use std::io::{Result as IoResult, Write};
use std::iter::repeat_with;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use rstest::rstest;
use secrecy::SecretString;
use tokio::task::{JoinSet, yield_now};
use tokio::time::sleep;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::context::{InteractionPollContext, SettlementFlag};
use super::host::{FetchFuture, FetchInteraction, SettlementFuture, SettlementHost};
use super::intervals::PollIntervals;
use super::poll::poll_background_cost;
use super::registry::{BackgroundPollRegistry, BackgroundPollRequest, maybe_schedule_poll};
use super::status::{
    is_pollable_interaction, is_pollable_status, is_terminal_status, missing_usage_is_expected,
};
use crate::types::json::Object;
use crate::types::response::InteractionsApiResponse;

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("{0}")]
struct FakeError(&'static str);

#[derive(Default)]
struct Recordings {
    billed: Mutex<Vec<String>>,
    released: Mutex<Vec<String>>,
}

#[derive(Clone, Default)]
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
        let result = if self.release_fails {
            Err(FakeError("release failed"))
        } else {
            Ok(())
        };
        Box::pin(ready(result))
    }
}

#[derive(Clone, Default)]
struct Script {
    answers: Arc<Mutex<VecDeque<Result<InteractionsApiResponse, FakeError>>>>,
    calls: Arc<AtomicUsize>,
}

impl Script {
    fn with_answers(answers: Vec<Result<InteractionsApiResponse, FakeError>>) -> Self {
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
                .unwrap_or_else(|| Ok(answer("in_progress", false)));
            Box::pin(async move { answer })
        })
    }
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

fn answer(status: &str, with_usage: bool) -> InteractionsApiResponse {
    let usage = if with_usage {
        r#","usage":{"total_tokens":5}"#
    } else {
        ""
    };
    let text = format!(r#"{{"id":"interaction-1","status":"{status}"{usage}}}"#);
    serde_json::from_str(&text).expect("the scripted response deserializes")
}

fn response_with(
    status: Option<&str>,
    id: Option<&str>,
    with_usage: bool,
) -> InteractionsApiResponse {
    InteractionsApiResponse {
        id: id.map(str::to_owned),
        object: None,
        model: None,
        agent: None,
        status: status.map(str::to_owned),
        created: None,
        updated: None,
        outputs: None,
        steps: None,
        usage: with_usage.then_some(Object::new()),
        extra: Object::new(),
        hidden_params: Object::new(),
    }
}

fn context(
    host: FakeHost,
    initial: u64,
    maximum: u64,
    timeout: u64,
) -> InteractionPollContext<FakeHost> {
    InteractionPollContext {
        interaction_id: "interaction-1".to_owned(),
        custom_llm_provider: "gemini".to_owned(),
        host,
        api_key: Some(SecretString::from("key")),
        api_base: None,
        initial_interval: Duration::from_secs(initial),
        max_interval: Duration::from_secs(maximum),
        timeout: Duration::from_secs(timeout),
        settlement: SettlementFlag::default(),
    }
}

#[derive(Clone, Copy)]
enum Failure {
    None,
    Billing,
    Release,
}

fn host_with(reservation: Option<&str>, failure: Failure) -> FakeHost {
    FakeHost {
        recordings: Arc::default(),
        reservation: reservation.map(str::to_owned),
        billing_fails: matches!(failure, Failure::Billing),
        release_fails: matches!(failure, Failure::Release),
    }
}

fn request(
    response: InteractionsApiResponse,
    host: Option<FakeHost>,
) -> BackgroundPollRequest<FakeHost> {
    BackgroundPollRequest {
        response,
        host,
        custom_llm_provider: "gemini".to_owned(),
        api_key: Some(SecretString::from("key")),
        api_base: Some("https://interactions.example".to_owned()),
        polling_enabled: true,
        initial_interval: Duration::from_secs(5),
        max_interval: Duration::from_secs(10),
        timeout: Duration::from_secs(30),
    }
}

#[rstest]
#[case::doubles_to_the_maximum(5, 20, 100, vec![5, 10, 20, 20, 20, 20])]
#[case::stops_at_the_timeout_boundary(5, 60, 75, vec![5, 10, 20, 40])]
#[case::drops_an_interval_that_would_pass_the_timeout(5, 60, 74, vec![5, 10, 20])]
#[case::an_initial_over_the_maximum_is_yielded_then_capped(30, 10, 100, vec![30, 10, 10, 10, 10, 10, 10, 10])]
#[case::a_short_timeout_yields_nothing(5, 60, 4, vec![])]
#[case::a_zero_initial_interval_yields_nothing(0, 60, 100, vec![])]
#[case::a_full_interval_saturates(u64::MAX, u64::MAX, u64::MAX, vec![u64::MAX])]
fn poll_intervals_double_to_the_maximum_and_stop_at_the_timeout(
    #[case] initial: u64,
    #[case] maximum: u64,
    #[case] timeout: u64,
    #[case] expected: Vec<u64>,
) {
    let intervals = PollIntervals {
        interval: Duration::from_secs(initial),
        maximum: Duration::from_secs(maximum),
        elapsed: Duration::ZERO,
        timeout: Duration::from_secs(timeout),
    };
    let collected: Vec<u64> = intervals.map(|interval| interval.as_secs()).collect();
    assert_eq!(collected, expected);
}

#[rstest]
#[case::completed("completed", true)]
#[case::failed("failed", true)]
#[case::cancelled("cancelled", true)]
#[case::incomplete("incomplete", true)]
#[case::budget_exceeded("budget_exceeded", true)]
#[case::requires_action("requires_action", true)]
#[case::in_progress("in_progress", false)]
#[case::queued("queued", false)]
#[case::unknown("teleported", false)]
fn terminal_status_classification_covers_every_status(#[case] wire: &str, #[case] terminal: bool) {
    assert_eq!(is_terminal_status(Some(wire)), terminal);
}

#[rstest]
#[case::in_progress("in_progress", true)]
#[case::queued("queued", true)]
#[case::completed("completed", false)]
#[case::failed("failed", false)]
#[case::cancelled("cancelled", false)]
#[case::incomplete("incomplete", false)]
#[case::budget_exceeded("budget_exceeded", false)]
#[case::requires_action("requires_action", false)]
#[case::unknown("teleported", false)]
fn pollable_status_classification_covers_every_status(#[case] wire: &str, #[case] pollable: bool) {
    assert_eq!(is_pollable_status(Some(wire)), pollable);
}

#[test]
fn an_absent_status_is_neither_terminal_nor_pollable() {
    assert!(!is_terminal_status(None));
    assert!(!is_pollable_status(None));
}

#[rstest]
#[case::in_progress_with_an_id("in_progress", "interaction-1", true)]
#[case::queued_with_an_id("queued", "interaction-1", true)]
#[case::completed_with_an_id("completed", "interaction-1", false)]
#[case::requires_action_with_an_id("requires_action", "interaction-1", false)]
#[case::in_progress_with_an_empty_id("in_progress", "", false)]
fn is_pollable_interaction_requires_a_pollable_status_and_an_id(
    #[case] status: &str,
    #[case] id: &str,
    #[case] pollable: bool,
) {
    let response = response_with(Some(status), Some(id), false);
    assert_eq!(is_pollable_interaction(&response), pollable);
}

#[test]
fn is_pollable_interaction_requires_a_present_status_and_id() {
    assert!(!is_pollable_interaction(&response_with(
        None,
        Some("interaction-1"),
        false
    )));
    assert!(!is_pollable_interaction(&response_with(
        Some("in_progress"),
        None,
        false
    )));
}

#[rstest]
#[case::in_progress("in_progress", true)]
#[case::queued("queued", true)]
#[case::failed("failed", true)]
#[case::cancelled("cancelled", true)]
#[case::incomplete("incomplete", true)]
#[case::budget_exceeded("budget_exceeded", true)]
#[case::completed("completed", false)]
#[case::requires_action("requires_action", false)]
#[case::unknown("teleported", true)]
fn missing_usage_is_expected_only_without_model_output(#[case] wire: &str, #[case] expected: bool) {
    let response = response_with(Some(wire), Some("interaction-1"), false);
    assert_eq!(missing_usage_is_expected(&response), expected);
}

#[test]
fn missing_usage_is_expected_when_the_status_is_absent() {
    assert!(missing_usage_is_expected(&response_with(
        None,
        Some("interaction-1"),
        false
    )));
}

#[test]
fn claim_settlement_succeeds_once_for_a_single_claimer() {
    let flag = SettlementFlag::default();
    assert!(!flag.settled.load(Ordering::Acquire));
    assert!(flag.claim_settlement());
    assert!(flag.settled.load(Ordering::Acquire));
    assert!(!flag.claim_settlement());
}

const RACE_ROUNDS: usize = 10_000;
const RACERS: usize = 2;

#[test]
fn settlement_is_claimed_by_exactly_one_of_two_racers() {
    (0..RACE_ROUNDS).for_each(|_| run_settlement_race());
}

fn run_settlement_race() {
    let flag = SettlementFlag::default();
    let barrier = Barrier::new(RACERS);
    let claims: usize = thread::scope(|scope| {
        let handles: Vec<_> = repeat_with(|| {
            scope.spawn(|| {
                barrier.wait();
                flag.claim_settlement()
            })
        })
        .take(RACERS)
        .collect();
        handles
            .into_iter()
            .map(|handle| usize::from(handle.join().expect("the racing claimer joins")))
            .sum()
    });
    assert_eq!(claims, 1);
    assert!(flag.settled.load(Ordering::Acquire));
}

#[tokio::test(start_paused = true)]
async fn polling_bills_a_terminal_status_with_usage_exactly_once() {
    let host = FakeHost::default();
    let script = Script::with_answers(vec![
        Ok(answer("in_progress", false)),
        Ok(answer("completed", true)),
    ]);
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(
        context(host.clone(), 5, 10, 30),
        script.fetch(),
    ));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    while tasks.join_next().await.is_some() {}
    assert_eq!(script.calls(), 2);
    assert_eq!(host.billed(), vec!["interaction-1".to_owned()]);
    assert!(host.released().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_requires_action_status_with_usage_is_billed_and_ends_the_poll() {
    let host = FakeHost::default();
    let script = Script::with_answers(vec![Ok(answer("requires_action", true))]);
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(
        context(host.clone(), 5, 10, 30),
        script.fetch(),
    ));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    while tasks.join_next().await.is_some() {}
    assert_eq!(script.calls(), 1);
    assert_eq!(host.billed(), vec!["interaction-1".to_owned()]);
    assert!(host.released().is_empty());
}

#[rstest]
#[case::completed("completed")]
#[case::failed("failed")]
#[case::cancelled("cancelled")]
#[case::incomplete("incomplete")]
#[case::budget_exceeded("budget_exceeded")]
#[case::requires_action("requires_action")]
#[tokio::test(start_paused = true)]
async fn a_terminal_status_without_usage_releases_the_reservation(#[case] wire: &str) {
    let host = host_with(Some("reservation-1"), Failure::None);
    let script = Script::with_answers(vec![Ok(answer(wire, false))]);
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(
        context(host.clone(), 5, 10, 30),
        script.fetch(),
    ));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    while tasks.join_next().await.is_some() {}
    assert_eq!(script.calls(), 1);
    assert!(host.billed().is_empty());
    assert_eq!(host.released(), vec!["reservation-1".to_owned()]);
}

#[tokio::test(start_paused = true)]
async fn a_pollable_status_with_usage_is_not_billed() {
    let host = host_with(Some("reservation-1"), Failure::None);
    let script = Script::with_answers(vec![Ok(answer("in_progress", true)); 3]);
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(
        context(host.clone(), 5, 10, 30),
        script.fetch(),
    ));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    while tasks.join_next().await.is_some() {}
    assert_eq!(script.calls(), 3);
    assert!(host.billed().is_empty());
    assert_eq!(host.released(), vec!["reservation-1".to_owned()]);
}

#[tokio::test(start_paused = true)]
async fn a_fetch_error_retries_on_the_next_interval() {
    let host = FakeHost::default();
    let script = Script::with_answers(vec![
        Err(FakeError("unreachable")),
        Ok(answer("completed", true)),
    ]);
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(
        context(host.clone(), 5, 10, 30),
        script.fetch(),
    ));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    while tasks.join_next().await.is_some() {}
    assert_eq!(script.calls(), 2);
    assert_eq!(host.billed(), vec!["interaction-1".to_owned()]);
}

#[tokio::test(start_paused = true)]
async fn polling_warns_when_the_timeout_expires_on_a_pollable_status() {
    let (logs, _guard) = capture_logs();
    let host = FakeHost::default();
    let script = Script::with_answers(Vec::new());
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(
        context(host.clone(), 5, 10, 30),
        script.fetch(),
    ));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    while tasks.join_next().await.is_some() {}
    let captured = logs.contents();
    assert!(captured.contains("gave up cost polling"));
    assert!(!captured.contains("in neither the pollable nor the terminal set"));
}

#[tokio::test(start_paused = true)]
async fn polling_reports_a_bad_state_when_the_last_status_is_unknown() {
    let (logs, _guard) = capture_logs();
    let host = FakeHost::default();
    let script = Script::with_answers(vec![Ok(answer("teleported", false)); 3]);
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(
        context(host.clone(), 5, 10, 30),
        script.fetch(),
    ));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    while tasks.join_next().await.is_some() {}
    assert_eq!(script.calls(), 3);
    assert!(
        logs.contents()
            .contains("in neither the pollable nor the terminal set")
    );
}

#[tokio::test(start_paused = true)]
async fn polling_warns_when_every_fetch_failed_until_the_timeout() {
    let (logs, _guard) = capture_logs();
    let host = FakeHost::default();
    let script = Script::with_answers(vec![Err(FakeError("unreachable")); 3]);
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(
        context(host.clone(), 5, 10, 30),
        script.fetch(),
    ));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    while tasks.join_next().await.is_some() {}
    assert_eq!(script.calls(), 3);
    let captured = logs.contents();
    assert!(captured.contains("will retry"));
    assert!(captured.contains("gave up cost polling"));
    assert!(!captured.contains("in neither the pollable nor the terminal set"));
}

#[tokio::test(start_paused = true)]
async fn settlement_won_by_the_fetcher_leaves_the_poll_nothing_to_bill() {
    let host = FakeHost::default();
    let fetch: FetchInteraction<FakeHost, FakeError> = Arc::new(move |context| {
        assert!(context.settlement.claim_settlement());
        let response = Ok(answer("completed", true));
        Box::pin(async move { response })
    });
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(
        context(host.clone(), 5, 10, 30),
        fetch,
    ));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    while tasks.join_next().await.is_some() {}
    assert!(host.billed().is_empty());
    assert!(host.released().is_empty());
}

#[tokio::test(start_paused = true)]
async fn settlement_stops_the_poll_task_before_its_next_fetch() {
    let host = FakeHost::default();
    let script = Script::with_answers(Vec::new());
    let mut registry = BackgroundPollRegistry::default();
    assert!(registry.schedule(context(host, 5, 10, 30), script.fetch()));
    yield_now().await;
    let poll = registry
        .active_poll("interaction-1")
        .expect("the poll is active");
    assert!(poll.settlement.claim_settlement());
    sleep(Duration::from_secs(6)).await;
    assert_eq!(script.calls(), 0);
    assert!(registry.active_poll("interaction-1").is_none());
    registry.shutdown().await;
}

const TIMEOUT_FETCHES: usize = 3;

#[tokio::test(start_paused = true)]
async fn settlement_won_elsewhere_skips_the_timeout_cleanup() {
    let (logs, _guard) = capture_logs();
    let host = host_with(Some("reservation-1"), Failure::None);
    let poll_context = context(host.clone(), 5, 10, 30);
    let settlement = poll_context.settlement.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let fetch: FetchInteraction<FakeHost, FakeError> = Arc::new(move |context| {
        let call = counted.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        if call == TIMEOUT_FETCHES {
            assert!(context.settlement.claim_settlement());
        }
        let response = Ok(answer("in_progress", false));
        Box::pin(async move { response })
    });
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(poll_context, fetch));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    while tasks.join_next().await.is_some() {}
    assert_eq!(calls.load(Ordering::SeqCst), TIMEOUT_FETCHES);
    assert!(settlement.settled.load(Ordering::Acquire));
    assert!(host.released().is_empty());
    assert!(!logs.contents().contains("gave up cost polling"));
}

#[tokio::test(start_paused = true)]
async fn a_billing_failure_releases_the_reservation_and_is_logged() {
    let (logs, _guard) = capture_logs();
    let host = host_with(Some("reservation-1"), Failure::Billing);
    let script = Script::with_answers(vec![Ok(answer("completed", true))]);
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(
        context(host.clone(), 5, 10, 30),
        script.fetch(),
    ));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    let result = tasks.join_next().await;
    assert!(matches!(result, Some(Ok(()))));
    assert_eq!(host.billed(), vec!["interaction-1".to_owned()]);
    assert_eq!(host.released(), vec!["reservation-1".to_owned()]);
    assert!(
        logs.contents()
            .contains("failed to bill a settled background interaction")
    );
}

#[tokio::test(start_paused = true)]
async fn a_release_failure_is_logged_and_swallowed() {
    let (logs, _guard) = capture_logs();
    let host = host_with(Some("reservation-1"), Failure::Release);
    let script = Script::with_answers(vec![Ok(answer("completed", false))]);
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(
        context(host.clone(), 5, 10, 30),
        script.fetch(),
    ));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    let result = tasks.join_next().await;
    assert!(matches!(result, Some(Ok(()))));
    assert_eq!(host.released(), vec!["reservation-1".to_owned()]);
    assert!(
        logs.contents().contains(
            "Failed to release budget reservation for an unbilled background interaction"
        )
    );
}

#[tokio::test(start_paused = true)]
async fn a_host_without_a_reservation_releases_nothing() {
    let host = FakeHost::default();
    let script = Script::with_answers(vec![Ok(answer("completed", false))]);
    let mut tasks = JoinSet::new();
    tasks.spawn(poll_background_cost(
        context(host.clone(), 5, 10, 30),
        script.fetch(),
    ));
    yield_now().await;
    sleep(Duration::from_secs(31)).await;
    while tasks.join_next().await.is_some() {}
    assert!(host.billed().is_empty());
    assert!(host.released().is_empty());
}

#[test]
fn maybe_schedule_poll_returns_false_outside_the_runtime() {
    let mut registry = BackgroundPollRegistry::default();
    let script = Script::with_answers(Vec::new());
    let poll_request = request(answer("in_progress", false), Some(FakeHost::default()));
    assert!(!maybe_schedule_poll(
        &mut registry,
        poll_request,
        script.fetch()
    ));
    assert_eq!(script.calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn maybe_schedule_poll_skips_a_disabled_polling_setting() {
    let mut registry = BackgroundPollRegistry::default();
    let script = Script::with_answers(Vec::new());
    let mut poll_request = request(answer("in_progress", false), Some(FakeHost::default()));
    poll_request.polling_enabled = false;
    assert!(!maybe_schedule_poll(
        &mut registry,
        poll_request,
        script.fetch()
    ));
    assert!(registry.active_poll("interaction-1").is_none());
}

#[tokio::test(start_paused = true)]
async fn maybe_schedule_poll_skips_a_non_pollable_status() {
    let mut registry = BackgroundPollRegistry::default();
    let script = Script::with_answers(Vec::new());
    let poll_request = request(answer("completed", true), Some(FakeHost::default()));
    assert!(!maybe_schedule_poll(
        &mut registry,
        poll_request,
        script.fetch()
    ));
    assert!(registry.active_poll("interaction-1").is_none());
}

#[tokio::test(start_paused = true)]
async fn maybe_schedule_poll_skips_a_pollable_status_without_a_host() {
    let mut registry = BackgroundPollRegistry::default();
    let script = Script::with_answers(Vec::new());
    let poll_request = request(answer("in_progress", false), None);
    assert!(!maybe_schedule_poll(
        &mut registry,
        poll_request,
        script.fetch()
    ));
    assert!(registry.active_poll("interaction-1").is_none());
}

#[tokio::test(start_paused = true)]
async fn maybe_schedule_poll_skips_a_status_without_an_id() {
    let mut registry = BackgroundPollRegistry::default();
    let script = Script::with_answers(Vec::new());
    let missing = request(
        response_with(Some("in_progress"), None, false),
        Some(FakeHost::default()),
    );
    assert!(!maybe_schedule_poll(&mut registry, missing, script.fetch()));
    let empty = request(
        response_with(Some("in_progress"), Some(""), false),
        Some(FakeHost::default()),
    );
    assert!(!maybe_schedule_poll(&mut registry, empty, script.fetch()));
    assert!(registry.active_poll("interaction-1").is_none());
}

#[tokio::test(start_paused = true)]
async fn maybe_schedule_poll_tracks_a_pollable_response() {
    let mut registry = BackgroundPollRegistry::default();
    let script = Script::with_answers(Vec::new());
    let poll_request = request(answer("in_progress", false), Some(FakeHost::default()));
    assert!(maybe_schedule_poll(
        &mut registry,
        poll_request,
        script.fetch()
    ));
    yield_now().await;
    let poll = registry
        .active_poll("interaction-1")
        .expect("the scheduled poll is tracked");
    assert_eq!(poll.custom_llm_provider, "gemini");
    assert_eq!(
        poll.api_base.as_deref(),
        Some("https://interactions.example")
    );
    assert!(poll.api_key.is_some());
    registry.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn active_poll_lookup_returns_the_context_while_the_poll_runs() {
    let mut registry = BackgroundPollRegistry::default();
    let script = Script::with_answers(Vec::new());
    assert!(registry.schedule(context(FakeHost::default(), 60, 60, 3600), script.fetch()));
    yield_now().await;
    let poll = registry
        .active_poll("interaction-1")
        .expect("the poll is active");
    assert_eq!(poll.custom_llm_provider, "gemini");
    assert!(poll.api_key.is_some());
    assert!(registry.active_poll("another-interaction").is_none());
    registry.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn active_poll_lookup_returns_none_after_the_poll_ends() {
    let mut registry = BackgroundPollRegistry::default();
    let script = Script::with_answers(vec![Ok(answer("completed", true))]);
    assert!(registry.schedule(context(FakeHost::default(), 5, 10, 30), script.fetch()));
    yield_now().await;
    sleep(Duration::from_secs(6)).await;
    assert_eq!(script.calls(), 1);
    assert!(registry.active_poll("interaction-1").is_none());
}

#[tokio::test(start_paused = true)]
async fn active_poll_lookup_keeps_the_newer_poll_when_the_older_one_ends() {
    let mut registry = BackgroundPollRegistry::default();
    let older = Script::with_answers(Vec::new());
    let mut older_context = context(FakeHost::default(), 1, 1, 1);
    older_context.api_base = Some("older".to_owned());
    assert!(registry.schedule(older_context, older.fetch()));
    let newer = Script::with_answers(Vec::new());
    let mut newer_context = context(FakeHost::default(), 60, 60, 3600);
    newer_context.api_base = Some("newer".to_owned());
    assert!(registry.schedule(newer_context, newer.fetch()));
    yield_now().await;
    sleep(Duration::from_secs(5)).await;
    let poll = registry
        .active_poll("interaction-1")
        .expect("the newer poll stays active");
    assert_eq!(poll.api_base.as_deref(), Some("newer"));
    registry.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn shutdown_cancels_running_polls() {
    let mut registry = BackgroundPollRegistry::default();
    let script = Script::with_answers(Vec::new());
    assert!(registry.schedule(context(FakeHost::default(), 60, 60, 3600), script.fetch()));
    yield_now().await;
    registry.shutdown().await;
    sleep(Duration::from_secs(61)).await;
    assert_eq!(script.calls(), 0);
    assert!(registry.active_poll("interaction-1").is_none());
}

#[tokio::test(start_paused = true)]
async fn a_schedule_reaps_finished_polls_so_the_task_set_stays_bounded() {
    let mut registry = BackgroundPollRegistry::default();
    let first = Script::with_answers(vec![Ok(answer("completed", true))]);
    assert!(registry.schedule(context(FakeHost::default(), 5, 10, 30), first.fetch()));
    yield_now().await;
    sleep(Duration::from_secs(6)).await;
    assert_eq!(first.calls(), 1);
    let second = Script::with_answers(Vec::new());
    assert!(registry.schedule(context(FakeHost::default(), 60, 60, 3600), second.fetch()));
    assert_eq!(registry.tasks.len(), 1);
    registry.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_panicked_poll_is_logged_when_the_next_schedule_reaps_it() {
    let (logs, _guard) = capture_logs();
    let mut registry = BackgroundPollRegistry::default();
    let panicking: FetchInteraction<FakeHost, FakeError> =
        Arc::new(|_context| -> FetchFuture<FakeError> {
            Box::pin(async { panic!("the poll panicked") })
        });
    assert!(registry.schedule(context(FakeHost::default(), 5, 10, 30), panicking));
    yield_now().await;
    sleep(Duration::from_secs(6)).await;
    let second = Script::with_answers(Vec::new());
    assert!(registry.schedule(context(FakeHost::default(), 60, 60, 3600), second.fetch()));
    assert!(
        logs.contents()
            .contains("background interaction poll ended with an error")
    );
    registry.shutdown().await;
}
