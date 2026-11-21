use betterlite_config::sections::settings::JsonValue;
use betterlite_core::routes::CallType;
use betterlite_logging::prometheus_latency::guardrail_series::{
    GuardrailObservation, GuardrailStatus,
};
use betterlite_openai::openai_base_config::Object as JsonMap;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{
    Answer, CLASS, FakeSeries, Harness, Observed, auth, capture_logs, fake, guardrail, harness, map,
};
use super::{NAME, context, http, metadata_field, reroute, rerouted};
use crate::guardrails::callback_caps::errors::ProxyHookError;
use crate::guardrails::callback_caps::traits::{PreCallReturn, ProxyCallback};
use crate::guardrails::hook_execution::execute::{GuardrailHook, HookAnswer, HookCall};
use crate::guardrails::hook_execution::metrics::{
    GuardrailMetrics as _, PrometheusGuardrails, run_guardrail_with_metrics,
};
use crate::guardrails::hook_execution::sensitive_route::handle_sensitive_data_route;

// each record's status and mode, in the order the records were written
fn records(data: &JsonMap) -> Vec<(JsonValue, JsonValue)> {
    let Some(JsonValue::Array(entries)) =
        metadata_field(data, "standard_logging_guardrail_information")
    else {
        return Vec::new();
    };
    entries
        .iter()
        .map(|entry| {
            (
                entry["guardrail_status"].clone(),
                entry["guardrail_mode"].clone(),
            )
        })
        .collect()
}

fn text(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

async fn execute(
    harness: &Harness,
    callback: &dyn ProxyCallback,
    hook: GuardrailHook,
    data: &mut JsonMap,
) -> Result<HookAnswer, ProxyHookError> {
    let user = auth();
    let call = HookCall {
        callback,
        hook,
        context: context(Some(&user), CallType::Acompletion),
    };
    harness.execution.execute_guardrail_hook(call, data).await
}

#[rstest]
#[case::lakera_pre_call("lakeraAI_Moderation", GuardrailHook::PreCall, Some("pre_call"))]
#[case::aporia_moderation("AporiaGuardrail", GuardrailHook::DuringCall, Some("during_call"))]
#[case::pangea_post_call(
    "PangeaHandler",
    GuardrailHook::PostCall(JsonValue::Null),
    Some("post_call")
)]
#[case::custom_pre_call("myCustomGuardrail", GuardrailHook::PreCall, Some("pre_call"))]
#[case::aporia_pre_call("AporiaGuardrail", GuardrailHook::PreCall, None)]
#[case::lakera_post_call("lakeraAI_Moderation", GuardrailHook::PostCall(JsonValue::Null), None)]
#[case::pangea_moderation("PangeaHandler", GuardrailHook::DuringCall, None)]
#[case::undecorated_class(CLASS, GuardrailHook::PreCall, None)]
#[tokio::test]
async fn a_decorated_own_hook_writes_one_invocation_record(
    #[case] class_name: &'static str,
    #[case] hook: GuardrailHook,
    #[case] mode: Option<&str>,
) {
    let harness = harness();
    let callback = fake(
        class_name,
        Some(guardrail(Some(NAME))),
        Answer::Pass,
        &harness.log,
    );
    let mut data = map(r#"{"model": "gpt"}"#);
    let answer = execute(&harness, &callback, hook, &mut data).await;
    assert_eq!(answer.map(|_answer| ()), Ok(()));
    let expected: Vec<_> = mode
        .map(|event| (text("success"), text(event)))
        .into_iter()
        .collect();
    assert_eq!(records(&data), expected);
    let spans = mode
        .map(|_event| "span:pii".to_owned())
        .into_iter()
        .collect::<Vec<_>>();
    assert_eq!(harness.spans.entries(), spans);
}

#[rstest]
#[case::post_call_answer(Answer::Pass, GuardrailHook::PostCall(JsonValue::Null), "\"checked\"")]
#[case::masked_messages(
    Answer::Replace(map(r#"{"messages": [{"role": "user", "content": "***"}]}"#)),
    GuardrailHook::PreCall,
    "\"mask\""
)]
#[case::same_messages(
    Answer::Replace(map(r#"{"messages": [{"role": "user", "content": "hi"}], "extra": 1}"#)),
    GuardrailHook::PreCall,
    "\"allow\""
)]
#[case::text_answer(Answer::Text("no".to_owned()), GuardrailHook::PreCall, "\"no\"")]
#[case::unchanged(Answer::Pass, GuardrailHook::PreCall, "{}")]
#[tokio::test]
async fn a_decorated_hook_records_what_it_returned(
    #[case] answer: Answer,
    #[case] hook: GuardrailHook,
    #[case] expected: &str,
) {
    let harness = harness();
    let callback = fake(
        "myCustomGuardrail",
        Some(guardrail(Some(NAME))),
        answer,
        &harness.log,
    );
    let mut data = map(r#"{"messages": [{"role": "user", "content": "hi"}]}"#);
    let outcome = execute(&harness, &callback, hook, &mut data).await;
    assert_eq!(outcome.map(|_answer| ()), Ok(()));
    let Some(JsonValue::Array(entries)) =
        metadata_field(&data, "standard_logging_guardrail_information")
    else {
        panic!("the hook wrote no record");
    };
    let responses: Vec<JsonValue> = entries
        .iter()
        .map(|entry| entry["guardrail_response"].clone())
        .collect();
    let parsed: JsonValue = serde_json::from_str(expected).unwrap();
    assert_eq!(responses, vec![parsed]);
}

#[rstest]
#[case::blocked(http("blocked"), "guardrail_intervened")]
#[case::server_error(ProxyHookError::Http { status: 500, detail: "down".to_owned() }, "guardrail_failed_to_respond")]
#[case::raised(ProxyHookError::GuardrailRaised { message: "no".to_owned(), guardrail_name: None, status: 400, blocked_content: false }, "guardrail_intervened")]
#[case::rerouted(rerouted(false), "guardrail_intervened")]
#[case::other(ProxyHookError::Other("down".to_owned()), "guardrail_failed_to_respond")]
#[tokio::test]
async fn a_decorated_hook_that_raises_records_its_failure(
    #[case] error: ProxyHookError,
    #[case] status: &str,
) {
    let harness = harness();
    let answer = Answer::Fail(error.clone());
    let callback = fake(
        "lakeraAI_Moderation",
        Some(guardrail(Some(NAME))),
        answer,
        &harness.log,
    );
    let mut data = map("{}");
    let outcome = execute(&harness, &callback, GuardrailHook::PreCall, &mut data).await;
    assert_eq!(outcome, Err(error));
    assert_eq!(records(&data), vec![(text(status), text("pre_call"))]);
}

#[tokio::test]
async fn a_decorated_class_on_the_unified_path_records_nothing_here() {
    let harness = harness();
    let mut callback = fake(
        "lakeraAI_Moderation",
        Some(guardrail(Some(NAME))),
        Answer::Pass,
        &harness.log,
    );
    callback.apply_guardrail = true;
    let mut data = map("{}");
    let answer = execute(&harness, &callback, GuardrailHook::PreCall, &mut data).await;
    assert_eq!(answer, Ok(HookAnswer::PreCall(PreCallReturn::Unchanged)));
    assert_eq!(records(&data), Vec::new());
    assert_eq!(
        harness.log.entries(),
        vec!["unified:pre_call:lakeraAI_Moderation".to_owned()]
    );
}

#[tokio::test]
async fn a_record_keeps_the_numbers_in_the_metadata() {
    let harness = harness();
    let callback = fake(
        "lakeraAI_Moderation",
        Some(guardrail(Some(NAME))),
        Answer::Pass,
        &harness.log,
    );
    let mut data = map(r#"{"metadata": {"count": 1, "ratio": 0.5, "big": 18446744073709551615}}"#);
    let answer = execute(&harness, &callback, GuardrailHook::DuringCall, &mut data).await;
    assert_eq!(answer, Ok(HookAnswer::DuringCall));
    assert_eq!(records(&data), vec![(text("success"), text("during_call"))]);
    let mut kept = data.get("metadata").and_then(JsonValue::as_object).cloned();
    if let Some(fields) = kept.as_mut() {
        fields.remove("standard_logging_guardrail_information");
    }
    let expected = map(r#"{"count": 1, "ratio": 0.5, "big": 18446744073709551615}"#);
    assert_eq!(kept, Some(expected));
}

#[test]
fn the_prometheus_logger_gets_the_three_series() {
    let metrics = PrometheusGuardrails {
        latency: FakeSeries::default(),
        requests: FakeSeries::default(),
        errors: FakeSeries::default(),
    };
    metrics.record(&GuardrailObservation {
        guardrail_name: NAME,
        latency_seconds: Decimal::ONE,
        status: GuardrailStatus::Error,
        error_type: Some("HTTPException"),
        hook_type: "pre_call",
    });
    assert_eq!(
        metrics.latency.0.entries(),
        vec!["pii,error,HTTPException,pre_call=1".to_owned()]
    );
    assert_eq!(
        metrics.requests.0.entries(),
        vec!["pii,error,pre_call".to_owned()]
    );
    assert_eq!(
        metrics.errors.0.entries(),
        vec!["pii,HTTPException,pre_call".to_owned()]
    );
}

#[tokio::test]
async fn a_post_call_without_a_guardrail_reports_under_its_class() {
    let harness = harness();
    let callback = fake(CLASS, None, Answer::Pass, &harness.log);
    let user = auth();
    let call = HookCall {
        callback: &callback,
        hook: GuardrailHook::PostCall(JsonValue::Null),
        context: context(Some(&user), CallType::Acompletion),
    };
    let mut data = map("{}");
    let answer = run_guardrail_with_metrics(&harness.execution, call, &mut data).await;
    assert_eq!(answer, Ok(HookAnswer::PostCall(Some(text("checked")))));
    let expected = Observed {
        name: CLASS.to_owned(),
        latency: Decimal::ONE,
        status: GuardrailStatus::Success,
        error_type: None,
        hook_type: "post_call".to_owned(),
    };
    assert_eq!(harness.metrics.observed(), vec![expected]);
}

#[tokio::test]
async fn a_raise_under_metrics_is_named_and_recorded() {
    let harness = harness();
    let raised = Answer::Fail(http(r#"{"error": "blocked", "code": 7}"#));
    let callback = fake(CLASS, Some(guardrail(Some(NAME))), raised, &harness.log);
    let user = auth();
    let call = HookCall {
        callback: &callback,
        hook: GuardrailHook::PostCall(JsonValue::Null),
        context: context(Some(&user), CallType::Acompletion),
    };
    let mut data = map(r#"{"metadata": {"count": 1}}"#);
    let answer = run_guardrail_with_metrics(&harness.execution, call, &mut data).await;
    let detail =
        r#"{"code":7,"error":"blocked","guardrail_mode":"pre_call","guardrail_name":"pii"}"#;
    assert_eq!(answer, Err(http(detail)));
    let expected = map(r#"{"metadata": {"count": 1, "applied_guardrails": ["pii"]}}"#);
    assert_eq!(data, expected);
}

#[derive(Debug, Clone, Copy)]
enum Hook {
    Registered,
    Missing,
}

#[rstest]
#[case::not_sticky(false, Hook::Registered, &[], false)]
#[case::sticky(true, Hook::Registered, &["session-1:onprem:pii"], false)]
#[case::sticky_without_the_hook(true, Hook::Missing, &[], true)]
#[case::not_sticky_without_the_hook(false, Hook::Missing, &[], false)]
#[tokio::test]
async fn a_sensitive_data_route_reroutes_and_stores_a_sticky_route_when_asked(
    #[case] sticky: bool,
    #[case] hook: Hook,
    #[case] stored: &[&str],
    #[case] warned: bool,
) {
    let (logs, _guard) = capture_logs();
    let mut harness = harness();
    if matches!(hook, Hook::Missing) {
        harness.execution.sensitive_routing = None;
    }
    let mut data = map(r#"{"model": "gpt", "metadata": {"user": "u"}}"#);
    let user = auth();
    handle_sensitive_data_route(&harness.execution, &reroute(sticky), &mut data, Some(&user)).await;
    let expected = map(
        r#"{"model": "onprem", "metadata": {"user": "u", "sensitive_data_routing_applied": true, "sensitive_data_routing_original_model": "gpt", "sensitive_data_routing_guardrail": "pii", "sensitive_data_routing_detection_info": {"entity": "SSN"}}}"#,
    );
    assert_eq!(data, expected);
    assert_eq!(harness.routes.entries(), stored);
    let warning = "the 'sensitive_data_routing' hook is not registered";
    assert_eq!(logs.contents().contains(warning), warned);
}

#[tokio::test]
async fn a_route_without_a_model_or_guardrail_writes_nulls() {
    let harness = harness();
    let mut route = reroute(false);
    route.guardrail_name = None;
    let mut data = map(r#"{"metadata": "text"}"#);
    handle_sensitive_data_route(&harness.execution, &route, &mut data, None).await;
    let expected = map(
        r#"{"model": "onprem", "metadata": {"sensitive_data_routing_applied": true, "sensitive_data_routing_original_model": null, "sensitive_data_routing_guardrail": null, "sensitive_data_routing_detection_info": {"entity": "SSN"}}}"#,
    );
    assert_eq!(data, expected);
}
