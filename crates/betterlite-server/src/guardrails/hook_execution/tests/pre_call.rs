use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_core::routes::CallType;
use betterlite_logging::prometheus_latency::guardrail_series::GuardrailStatus;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{
    Answer, CLASS, Fake, FakeBalancer, Gate, Harness, Observed, auth, capture_logs, fake,
    guardrail, harness, map,
};
use super::{Class, NAME, context, http, metadata_field, rerouted, shaped, strings};
use crate::guardrails::callback_caps::errors::{
    GuardrailGateError, ModifiedResponseError, ProxyException, ProxyHookError,
};
use crate::guardrails::callback_caps::traits::{GuardrailEvent, ProxyCallback};
use crate::guardrails::hook_execution::pre_call_runners::{
    CallbackRun, process_guardrail_callback, run_parallel, run_sequential,
};

fn observed(name: &str, status: GuardrailStatus, error_type: Option<&str>) -> Observed {
    Observed {
        name: name.to_owned(),
        latency: Decimal::ONE,
        status,
        error_type: error_type.map(str::to_owned),
        hook_type: "pre_call".to_owned(),
    }
}

fn events(callback: &Fake) -> Vec<GuardrailEvent> {
    callback.guardrail.as_ref().map_or_else(Vec::new, |held| {
        held.knobs
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    })
}

fn proxy_exception() -> ProxyHookError {
    ProxyHookError::from(ProxyException {
        message: "denied".to_owned(),
        error_type: "auth_error".to_owned(),
        param: None,
        code: "401".to_owned(),
        headers: BTreeMap::new(),
    })
}

fn modified() -> ProxyHookError {
    let response = ModifiedResponseError {
        message: "masked".to_owned(),
        model: "gpt".to_owned(),
        guardrail_name: Some(NAME.to_owned()),
        detection_info: BTreeMap::new(),
        original_response: None,
    };
    ProxyHookError::ModifyResponse(Box::new(response))
}

fn raised() -> ProxyHookError {
    ProxyHookError::GuardrailRaised {
        message: "raised".to_owned(),
        guardrail_name: Some(NAME.to_owned()),
        status: 400,
        blocked_content: true,
    }
}

#[tokio::test]
async fn a_load_balanced_name_runs_the_pick_and_reports_the_listed_guardrail() {
    let mut harness = harness();
    let picked: Arc<dyn ProxyCallback> = Arc::new(fake("Picked", None, Answer::Pass, &harness.log));
    harness.execution.balancer = Arc::new(FakeBalancer {
        balanced: true,
        pick: Some(picked),
    });
    let listed = shaped(Class::Plain, Answer::Pass, &harness);
    let user = auth();
    let mut data = map("{}");
    let run = process_guardrail_callback(
        &harness.execution,
        &listed,
        &mut data,
        context(Some(&user), CallType::Acompletion),
    )
    .await;
    assert_eq!(run, Ok(CallbackRun::Ran));
    assert_eq!(harness.log.entries(), vec!["Picked:pre_call".to_owned()]);
    let markers = metadata_field(&data, "_pre_call_executed_guardrails");
    assert_eq!(markers, Some(&strings(&["ran:pii"])));
    let expected = observed(NAME, GuardrailStatus::Success, None);
    assert_eq!(harness.metrics.observed(), vec![expected]);
}

#[tokio::test]
async fn a_balancer_without_a_pick_fails_as_an_exception() {
    let mut harness = harness();
    harness.execution.balancer = Arc::new(FakeBalancer {
        balanced: true,
        pick: None,
    });
    let listed = shaped(Class::Plain, Answer::Pass, &harness);
    let user = auth();
    let mut data = map("{}");
    let run = process_guardrail_callback(
        &harness.execution,
        &listed,
        &mut data,
        context(Some(&user), CallType::Acompletion),
    )
    .await;
    let message = "No callback found for guardrail: pii".to_owned();
    assert_eq!(run, Err(ProxyHookError::Other(message)));
    assert_eq!(harness.log.entries(), Vec::<String>::new());
    let expected = observed(NAME, GuardrailStatus::Error, Some("Exception"));
    assert_eq!(harness.metrics.observed(), vec![expected]);
}

#[tokio::test]
async fn a_guardrail_without_a_name_skips_balancing_and_reports_unknown() {
    let mut harness = harness();
    harness.execution.balancer = Arc::new(FakeBalancer {
        balanced: true,
        pick: None,
    });
    let listed = fake(CLASS, Some(guardrail(None)), Answer::Pass, &harness.log);
    let user = auth();
    let mut data = map("{}");
    let run = process_guardrail_callback(
        &harness.execution,
        &listed,
        &mut data,
        context(Some(&user), CallType::Acompletion),
    )
    .await;
    assert_eq!(run, Ok(CallbackRun::Ran));
    assert_eq!(
        harness.log.entries(),
        vec!["TestGuardrail:pre_call".to_owned()]
    );
    let expected = observed("unknown", GuardrailStatus::Success, None);
    assert_eq!(harness.metrics.observed(), vec![expected]);
}

#[rstest]
#[case::call_mcp_tool(CallType::CallMcpTool, GuardrailEvent::PreMcpCall)]
#[case::list_mcp_tools(CallType::ListMcpTools, GuardrailEvent::PreMcpCall)]
#[case::chat(CallType::Acompletion, GuardrailEvent::PreCall)]
#[tokio::test]
async fn the_pre_call_gate_hears_the_mcp_event_for_mcp_calls(
    #[case] call_type: CallType,
    #[case] expected: GuardrailEvent,
) {
    let harness = harness();
    let callback = shaped(Class::Plain, Answer::Pass, &harness);
    let user = auth();
    let mut data = map("{}");
    let run = process_guardrail_callback(
        &harness.execution,
        &callback,
        &mut data,
        context(Some(&user), call_type),
    )
    .await;
    assert_eq!(run, Ok(CallbackRun::Ran));
    assert_eq!(events(&callback), vec![expected]);
}

#[rstest]
#[case::gate_skip(Gate::Skip, Ok(CallbackRun::Skipped))]
#[case::gate_error(Gate::Fail(GuardrailGateError::TagMode), Err(ProxyHookError::Other(GuardrailGateError::TagMode.to_string())))]
#[tokio::test]
async fn a_gate_that_is_not_true_skips_without_metrics(
    #[case] gate: Gate,
    #[case] expected: Result<CallbackRun, ProxyHookError>,
) {
    let harness = harness();
    let mut held = guardrail(Some(NAME));
    held.knobs.gate = gate;
    let callback = fake(CLASS, Some(held), Answer::Pass, &harness.log);
    let user = auth();
    let mut data = map("{}");
    let run = process_guardrail_callback(
        &harness.execution,
        &callback,
        &mut data,
        context(Some(&user), CallType::Acompletion),
    )
    .await;
    assert_eq!(run, expected);
    assert_eq!(data, map("{}"));
    assert_eq!(harness.log.entries(), Vec::<String>::new());
    assert_eq!(harness.metrics.observed(), Vec::<Observed>::new());
}

#[tokio::test]
async fn a_callback_that_is_no_guardrail_is_skipped() {
    let harness = harness();
    let callback = fake(CLASS, None, Answer::Pass, &harness.log);
    let user = auth();
    let mut data = map("{}");
    let run = process_guardrail_callback(
        &harness.execution,
        &callback,
        &mut data,
        context(Some(&user), CallType::Acompletion),
    )
    .await;
    assert_eq!(run, Ok(CallbackRun::Skipped));
    assert_eq!(harness.log.entries(), Vec::<String>::new());
}

#[rstest]
#[case::passed(Answer::Pass, GuardrailStatus::Success, None)]
#[case::rerouted(Answer::Fail(rerouted(false)), GuardrailStatus::Intervened, None)]
#[case::http(
    Answer::Fail(http("blocked")),
    GuardrailStatus::Error,
    Some("HTTPException")
)]
#[case::proxy(
    Answer::Fail(proxy_exception()),
    GuardrailStatus::Error,
    Some("ProxyException")
)]
#[case::raised(
    Answer::Fail(raised()),
    GuardrailStatus::Error,
    Some("GuardrailRaisedException")
)]
#[case::modified(
    Answer::Fail(modified()),
    GuardrailStatus::Error,
    Some("ModifyResponseException")
)]
#[case::rejected(Answer::Text("no".to_owned()), GuardrailStatus::Error, Some("RejectedRequestError"))]
#[case::other(Answer::Fail(ProxyHookError::Other("down".to_owned())), GuardrailStatus::Error, Some("Exception"))]
#[tokio::test]
async fn every_run_reports_its_status_and_latency(
    #[case] answer: Answer,
    #[case] status: GuardrailStatus,
    #[case] error_type: Option<&str>,
) {
    let harness = harness();
    let callback = shaped(Class::Plain, answer, &harness);
    let user = auth();
    let mut data = map("{}");
    let run = process_guardrail_callback(
        &harness.execution,
        &callback,
        &mut data,
        context(Some(&user), CallType::Completion),
    )
    .await;
    assert_eq!(run.is_ok(), status == GuardrailStatus::Success);
    let expected = observed(NAME, status, error_type);
    assert_eq!(harness.metrics.observed(), vec![expected]);
    let markers = metadata_field(&data, "_pre_call_executed_guardrails");
    let marked = (status == GuardrailStatus::Success).then(|| strings(&["ran:pii"]));
    assert_eq!(markers, marked.as_ref());
}

#[tokio::test]
async fn no_prometheus_logger_reports_nothing() {
    let mut harness = harness();
    harness.execution.metrics = None;
    let callback = shaped(Class::Plain, Answer::Pass, &harness);
    let user = auth();
    let mut data = map("{}");
    let run = process_guardrail_callback(
        &harness.execution,
        &callback,
        &mut data,
        context(Some(&user), CallType::Acompletion),
    )
    .await;
    assert_eq!(run, Ok(CallbackRun::Ran));
    assert_eq!(harness.metrics.observed(), Vec::<Observed>::new());
}

#[rstest]
#[case::added(
    r#"{"error": "blocked"}"#,
    r#"{"error":"blocked","guardrail_mode":"pre_call","guardrail_name":"pii"}"#
)]
#[case::kept(
    r#"{"error": "blocked", "guardrail_name": "mine", "guardrail_mode": "x"}"#,
    r#"{"error":"blocked","guardrail_mode":"x","guardrail_name":"mine"}"#
)]
#[case::text("blocked", "blocked")]
#[case::list("[1]", "[1]")]
#[tokio::test]
async fn an_http_error_with_an_object_detail_names_the_guardrail(
    #[case] detail: &str,
    #[case] expected: &str,
) {
    let harness = harness();
    let callback = shaped(Class::Plain, Answer::Fail(http(detail)), &harness);
    let user = auth();
    let mut data = map("{}");
    let run = process_guardrail_callback(
        &harness.execution,
        &callback,
        &mut data,
        context(Some(&user), CallType::Acompletion),
    )
    .await;
    assert_eq!(run, Err(http(expected)));
}

#[rstest]
#[case::edited_snapshot(Answer::Edit, Some(r#"{"model": "raw"}"#), r#"{"model": "raw"}"#, true)]
#[case::passed_snapshot(
    Answer::Pass,
    Some(r#"{"model": "raw"}"#),
    r#"{"model": "raw"}"#,
    false
)]
#[case::edited_without_snapshot(Answer::Edit, None, r#"{"model": "live"}"#, true)]
#[case::replaced_snapshot(
    Answer::Replace(map(r#"{"model": "swapped"}"#)),
    Some(r#"{"model": "raw"}"#),
    r#"{"model": "raw"}"#,
    true
)]
#[tokio::test]
async fn a_scan_raw_request_guardrail_reads_the_snapshot_and_its_edits_are_dropped(
    #[case] answer: Answer,
    #[case] snapshot: Option<&str>,
    #[case] expected_seen: &str,
    #[case] warned: bool,
) {
    let (logs, _guard) = capture_logs();
    let harness = harness();
    let mut callback = shaped(Class::Plain, answer, &harness);
    if let Some(held) = callback.guardrail.as_mut() {
        held.knobs.scan_raw_request = true;
    }
    let raw = snapshot.map(map);
    let user = auth();
    let mut data = map(r#"{"model": "live"}"#);
    let scope = context(Some(&user), CallType::Acompletion);
    let outcome = run_sequential(
        &harness.execution,
        &callback,
        &mut data,
        raw.as_ref(),
        scope,
    )
    .await;
    assert_eq!(outcome, Ok(()));
    assert_eq!(callback.seen(), vec![map(expected_seen)]);
    let expected =
        map(r#"{"model": "live", "metadata": {"_pre_call_executed_guardrails": ["ran:pii"]}}"#);
    assert_eq!(data, expected);
    let warning = "Guardrail 'pii' has scan_raw_request=True but returned a modified payload";
    assert_eq!(logs.contents().contains(warning), warned);
}

#[tokio::test]
async fn a_skipped_scan_raw_request_guardrail_leaves_no_mark() {
    let harness = harness();
    let mut held = guardrail(Some(NAME));
    held.knobs.scan_raw_request = true;
    held.knobs.gate = Gate::Skip;
    let callback = fake(CLASS, Some(held), Answer::Pass, &harness.log);
    let user = auth();
    let mut data = map("{}");
    let scope = context(Some(&user), CallType::Acompletion);
    let outcome = run_sequential(&harness.execution, &callback, &mut data, None, scope).await;
    assert_eq!(outcome, Ok(()));
    assert_eq!(data, map("{}"));
}

#[rstest]
#[case::edited(Answer::Edit, r#"{"model": "live", "edited": "TestGuardrail", "metadata": {"_pre_call_executed_guardrails": ["ran:pii"]}}"#)]
#[case::replaced(
    Answer::Replace(map(r#"{"model": "swapped"}"#)),
    r#"{"model": "swapped", "metadata": {"_pre_call_executed_guardrails": ["ran:pii"]}}"#
)]
#[tokio::test]
async fn a_sequential_guardrail_edits_the_live_request(
    #[case] answer: Answer,
    #[case] expected: &str,
) {
    let harness = harness();
    let callback = shaped(Class::Plain, answer, &harness);
    let user = auth();
    let mut data = map(r#"{"model": "live"}"#);
    let scope = context(Some(&user), CallType::Acompletion);
    let raw = map(r#"{"model": "raw"}"#);
    let outcome = run_sequential(&harness.execution, &callback, &mut data, Some(&raw), scope).await;
    assert_eq!(outcome, Ok(()));
    assert_eq!(callback.seen(), vec![map(r#"{"model": "live"}"#)]);
    assert_eq!(data, map(expected));
}

#[rstest]
#[case::blocked(http("blocked"), false, Some(strings(&["pii"])))]
#[case::blocked_on_the_snapshot(http("blocked"), true, Some(strings(&["pii"])))]
#[case::rerouted(rerouted(false), false, None)]
#[case::rerouted_on_the_snapshot(rerouted(false), true, None)]
#[tokio::test]
async fn a_sequential_raise_records_the_guardrail_but_a_reroute_does_not(
    #[case] error: ProxyHookError,
    #[case] scan_raw_request: bool,
    #[case] expected: Option<JsonValue>,
) {
    let harness = harness();
    let mut callback = shaped(Class::Plain, Answer::Fail(error.clone()), &harness);
    if let Some(held) = callback.guardrail.as_mut() {
        held.knobs.scan_raw_request = scan_raw_request;
    }
    let user = auth();
    let mut data = map("{}");
    let scope = context(Some(&user), CallType::Acompletion);
    let outcome = run_sequential(&harness.execution, &callback, &mut data, None, scope).await;
    assert_eq!(outcome, Err(error));
    assert_eq!(
        metadata_field(&data, "applied_guardrails"),
        expected.as_ref()
    );
}

fn parallel(answers: Vec<(Answer, u64)>, harness: &Harness) -> Vec<Arc<dyn ProxyCallback>> {
    answers
        .into_iter()
        .map(|(answer, delay)| -> Arc<dyn ProxyCallback> {
            let mut callback = shaped(Class::Plain, answer, harness);
            callback.delay = Duration::from_millis(delay);
            Arc::new(callback)
        })
        .collect()
}

#[rstest]
#[case::slow_block_after_fast_reroute(vec![(Answer::Fail(rerouted(false)), 0), (Answer::Fail(http("blocked")), 50)], http("blocked"))]
#[case::two_blocks(vec![(Answer::Fail(http("first")), 50), (Answer::Fail(http("second")), 0)], http("first"))]
#[case::two_reroutes(vec![(Answer::Fail(rerouted(false)), 50), (Answer::Fail(rerouted(true)), 0)], rerouted(false))]
#[case::one_block(vec![(Answer::Pass, 0), (Answer::Fail(http("blocked")), 0)], http("blocked"))]
#[tokio::test(start_paused = true)]
async fn parallel_guardrails_raise_a_block_before_a_reroute(
    #[case] answers: Vec<(Answer, u64)>,
    #[case] expected: ProxyHookError,
) {
    let harness = harness();
    let guardrails = parallel(answers, &harness);
    let user = auth();
    let mut data = map("{}");
    let scope = context(Some(&user), CallType::Acompletion);
    let outcome = run_parallel(&harness.execution, &guardrails, &mut data, None, scope).await;
    assert_eq!(outcome, Err(expected));
    assert_eq!(harness.log.entries().len(), 2);
}

#[tokio::test]
async fn parallel_guardrails_merge_their_marks_and_drop_their_edits() {
    let harness = harness();
    let first = Arc::new(fake(
        CLASS,
        Some(guardrail(Some("first"))),
        Answer::Edit,
        &harness.log,
    ));
    let mut held = guardrail(Some("raw"));
    held.knobs.scan_raw_request = true;
    let scanner = Arc::new(fake("Scanner", Some(held), Answer::Edit, &harness.log));
    let guardrails: Vec<Arc<dyn ProxyCallback>> =
        vec![Arc::<Fake>::clone(&first), Arc::<Fake>::clone(&scanner)];
    let user = auth();
    let raw = map(r#"{"model": "raw", "metadata": {"from": "snapshot"}}"#);
    let mut data = map(r#"{"model": "live"}"#);
    let scope = context(Some(&user), CallType::Acompletion);
    let outcome = run_parallel(
        &harness.execution,
        &guardrails,
        &mut data,
        Some(&raw),
        scope,
    )
    .await;
    assert_eq!(outcome, Ok(()));
    assert_eq!(first.seen(), vec![map(r#"{"model": "live"}"#)]);
    assert_eq!(scanner.seen(), vec![raw]);
    let expected = map(
        r#"{"model": "live", "metadata": {"_pre_call_executed_guardrails": ["ran:first", "ran:raw"]}}"#,
    );
    assert_eq!(data, expected);
}

#[tokio::test]
async fn a_parallel_raise_is_recorded_on_the_live_request() {
    let harness = harness();
    let answers = vec![
        (Answer::Fail(http("blocked")), 0),
        (Answer::Fail(rerouted(false)), 0),
    ];
    let guardrails = parallel(answers, &harness);
    let user = auth();
    let mut data = map("{}");
    let scope = context(Some(&user), CallType::Acompletion);
    let outcome = run_parallel(&harness.execution, &guardrails, &mut data, None, scope).await;
    assert_eq!(outcome, Err(http("blocked")));
    let applied = metadata_field(&data, "applied_guardrails");
    assert_eq!(applied, Some(&strings(&["pii"])));
}
