#![cfg(test)]

mod during_call;
mod pre_call;
mod recording;
mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_startup::router_list::RouterGuardrail;
use betterlite_openai::openai_base_config::Object as JsonMap;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;

use self::support::{Answer, CLASS, Fake, Harness, auth, fake, guardrail, harness, map};
use super::balancer::RouterGuardrailBalancer;
use super::bridge::merge_metadata;
use super::execute::{
    CallContext, GuardrailBalancer as _, GuardrailHook, HookAnswer, HookCall, HookPath, hook_path,
    process_pre_call_response,
};
use crate::guardrails::callback_caps::errors::{ProxyHookError, SensitiveRouteError};
use crate::guardrails::callback_caps::traits::{PreCallReturn, ProxyCallback};

const NAME: &str = "pii";

#[derive(Debug, Clone, Copy)]
enum Class {
    Plain,
    Unified,
    NativeLifecycle,
    NativeDuringCall,
}

fn shaped(class: Class, answer: Answer, harness: &Harness) -> Fake {
    let mut held = guardrail(Some(NAME));
    held.knobs.native_during_call = matches!(class, Class::NativeDuringCall);
    let mut callback = fake(CLASS, Some(held), answer, &harness.log);
    callback.apply_guardrail = !matches!(class, Class::Plain);
    callback.native_lifecycle = matches!(class, Class::NativeLifecycle);
    callback
}

fn http(detail: &str) -> ProxyHookError {
    ProxyHookError::Http {
        status: 400,
        detail: detail.to_owned(),
    }
}

fn reroute(sticky_session_routing: bool) -> SensitiveRouteError {
    SensitiveRouteError {
        session_id: "session-1".to_owned(),
        route_to_model: "onprem".to_owned(),
        guardrail_name: Some(NAME.to_owned()),
        sticky_session_routing,
        detection_info: BTreeMap::from([(
            "entity".to_owned(),
            JsonValue::String("SSN".to_owned()),
        )]),
    }
}

fn rerouted(sticky_session_routing: bool) -> ProxyHookError {
    ProxyHookError::SensitiveDataRoute(Box::new(reroute(sticky_session_routing)))
}

fn metadata_field<'data>(data: &'data JsonMap, field: &str) -> Option<&'data JsonValue> {
    data.get("metadata")
        .and_then(JsonValue::as_object)
        .and_then(|bucket| bucket.get(field))
}

fn strings(values: &[&str]) -> JsonValue {
    JsonValue::Array(
        values
            .iter()
            .map(|text| JsonValue::String((*text).to_owned()))
            .collect(),
    )
}

const fn context(
    user_api_key_dict: Option<&UserAPIKeyAuth>,
    call_type: CallType,
) -> CallContext<'_> {
    CallContext {
        user_api_key_dict,
        call_type,
    }
}

#[rstest]
#[case::plain_pre_call(Class::Plain, GuardrailHook::PreCall, HookPath::Own)]
#[case::plain_during_call(Class::Plain, GuardrailHook::DuringCall, HookPath::Own)]
#[case::unified_pre_call(Class::Unified, GuardrailHook::PreCall, HookPath::Unified)]
#[case::unified_during_call(Class::Unified, GuardrailHook::DuringCall, HookPath::Unified)]
#[case::unified_post_call(
    Class::Unified,
    GuardrailHook::PostCall(JsonValue::Null),
    HookPath::Unified
)]
#[case::native_lifecycle(Class::NativeLifecycle, GuardrailHook::PreCall, HookPath::Own)]
#[case::native_lifecycle_post_call(
    Class::NativeLifecycle,
    GuardrailHook::PostCall(JsonValue::Null),
    HookPath::Own
)]
#[case::native_during_call_hook(Class::NativeDuringCall, GuardrailHook::DuringCall, HookPath::Own)]
#[case::native_during_call_on_pre_call(
    Class::NativeDuringCall,
    GuardrailHook::PreCall,
    HookPath::Unified
)]
#[case::native_during_call_on_post_call(
    Class::NativeDuringCall,
    GuardrailHook::PostCall(JsonValue::Null),
    HookPath::Unified
)]
fn hook_path_follows_the_litellm_rule(
    #[case] class: Class,
    #[case] hook: GuardrailHook,
    #[case] expected: HookPath,
) {
    let harness = harness();
    let callback = shaped(class, Answer::Pass, &harness);
    let user = auth();
    let scope = context(Some(&user), CallType::Acompletion);
    assert_eq!(hook_path(&callback, &hook, scope), expected);
}

#[test]
fn a_unified_callback_without_a_guardrail_keeps_the_unified_during_call() {
    let harness = harness();
    let mut callback = fake(CLASS, None, Answer::Pass, &harness.log);
    callback.apply_guardrail = true;
    let user = auth();
    let scope = context(Some(&user), CallType::Acompletion);
    assert_eq!(
        hook_path(&callback, &GuardrailHook::DuringCall, scope),
        HookPath::Unified
    );
}

#[test]
fn a_unified_during_call_without_the_callers_key_takes_the_own_hook() {
    let harness = harness();
    let callback = shaped(Class::Unified, Answer::Pass, &harness);
    let scope = context(None, CallType::Acompletion);
    assert_eq!(
        hook_path(&callback, &GuardrailHook::DuringCall, scope),
        HookPath::Own
    );
}

#[rstest]
#[case::unified_pre_call(
    Class::Unified,
    GuardrailHook::PreCall,
    "unified:pre_call:TestGuardrail",
    HookAnswer::PreCall(PreCallReturn::Unchanged)
)]
#[case::unified_moderation(
    Class::Unified,
    GuardrailHook::DuringCall,
    "unified:moderation:TestGuardrail",
    HookAnswer::DuringCall
)]
#[case::unified_post_call(Class::Unified, GuardrailHook::PostCall(JsonValue::String("reply".to_owned())), "unified:post_call:TestGuardrail", HookAnswer::PostCall(Some(JsonValue::String("reply".to_owned()))))]
#[case::own_pre_call(
    Class::Plain,
    GuardrailHook::PreCall,
    "TestGuardrail:pre_call",
    HookAnswer::PreCall(PreCallReturn::Unchanged)
)]
#[case::own_moderation(
    Class::Plain,
    GuardrailHook::DuringCall,
    "TestGuardrail:moderation",
    HookAnswer::DuringCall
)]
#[case::own_post_call(Class::Plain, GuardrailHook::PostCall(JsonValue::String("reply".to_owned())), "TestGuardrail:post_call", HookAnswer::PostCall(Some(JsonValue::String("checked".to_owned()))))]
#[tokio::test]
async fn execute_runs_the_unified_or_the_own_hook(
    #[case] class: Class,
    #[case] hook: GuardrailHook,
    #[case] expected_call: &str,
    #[case] expected_answer: HookAnswer,
) {
    let harness = harness();
    let callback = shaped(class, Answer::Pass, &harness);
    let user = auth();
    let call = HookCall {
        callback: &callback,
        hook,
        context: context(Some(&user), CallType::Acompletion),
    };
    let mut data = map(r#"{"model": "gpt"}"#);
    let answer = harness
        .execution
        .execute_guardrail_hook(call, &mut data)
        .await;
    assert_eq!(answer, Ok(expected_answer));
    assert_eq!(harness.log.entries(), vec![expected_call.to_owned()]);
}

#[tokio::test]
async fn an_own_pre_call_hook_needs_the_callers_key() {
    let harness = harness();
    let callback = shaped(Class::Plain, Answer::Pass, &harness);
    let call = HookCall {
        callback: &callback,
        hook: GuardrailHook::PreCall,
        context: context(None, CallType::Acompletion),
    };
    let mut data = map("{}");
    let answer = harness
        .execution
        .execute_guardrail_hook(call, &mut data)
        .await;
    let expected = "the pre-call hook of TestGuardrail needs the caller's key";
    assert_eq!(answer, Err(ProxyHookError::Other(expected.to_owned())));
    assert_eq!(harness.log.entries(), Vec::<String>::new());
}

#[rstest]
#[case::completion(CallType::Completion, r#"{"model": "gpt"}"#, ProxyHookError::RejectedRequest { message: "no".to_owned(), model: "gpt".to_owned() })]
#[case::text_completion_without_model(CallType::TextCompletion, "{}", ProxyHookError::RejectedRequest { message: "no".to_owned(), model: String::new() })]
#[case::acompletion(
    CallType::Acompletion,
    r#"{"model": "gpt"}"#,
    http(r#"{"error":"no"}"#)
)]
#[case::atext_completion(CallType::AtextCompletion, "{}", http(r#"{"error":"no"}"#))]
#[case::embedding(CallType::Embedding, "{}", http(r#"{"error":"no"}"#))]
fn a_text_answer_rejects_the_request(
    #[case] call_type: CallType,
    #[case] request: &str,
    #[case] expected: ProxyHookError,
) {
    let mut data = map(request);
    let outcome =
        process_pre_call_response(PreCallReturn::Text("no".to_owned()), &mut data, call_type);
    assert_eq!(outcome, Err(expected));
}

#[test]
fn a_map_answer_replaces_the_data_and_unchanged_keeps_it() {
    let mut data = map(r#"{"model": "gpt"}"#);
    process_pre_call_response(PreCallReturn::Unchanged, &mut data, CallType::Completion).unwrap();
    assert_eq!(data, map(r#"{"model": "gpt"}"#));
    let replacement = map(r#"{"model": "other"}"#);
    process_pre_call_response(
        PreCallReturn::Data(replacement),
        &mut data,
        CallType::Completion,
    )
    .unwrap();
    assert_eq!(data, map(r#"{"model": "other"}"#));
}

fn deployment(guardrail_name: &str, id: Option<&str>) -> RouterGuardrail {
    RouterGuardrail {
        guardrail_name: guardrail_name.to_owned(),
        id: id.map(str::to_owned),
        guardrail: "presidio".to_owned(),
        mode: LitellmParamsMode::One("pre_call".to_owned()),
        api_key: None,
        api_base: None,
    }
}

fn balancer(
    guardrails: Vec<RouterGuardrail>,
    harness: &Harness,
) -> RouterGuardrailBalancer<StdRng> {
    let first: Arc<dyn ProxyCallback> = Arc::new(fake("First", None, Answer::Pass, &harness.log));
    let second: Arc<dyn ProxyCallback> = Arc::new(fake("Second", None, Answer::Pass, &harness.log));
    RouterGuardrailBalancer {
        guardrails,
        callbacks_by_id: BTreeMap::from([("a".to_owned(), first), ("b".to_owned(), second)]),
        rng: Mutex::new(StdRng::seed_from_u64(7)),
    }
}

#[rstest]
#[case::no_deployment(&[], false)]
#[case::one_deployment(&["pii"], false)]
#[case::two_deployments(&["pii", "pii"], true)]
#[case::three_deployments(&["pii", "toxic", "pii", "pii"], true)]
#[case::other_names(&["pii", "toxic"], false)]
fn load_balancing_needs_more_than_one_deployment(#[case] names: &[&str], #[case] expected: bool) {
    let harness = harness();
    let guardrails = names
        .iter()
        .map(|name| deployment(name, Some("a")))
        .collect();
    let router = balancer(guardrails, &harness);
    assert_eq!(router.should_use_load_balancing(NAME), expected);
}

#[rstest]
#[case::listed(vec![deployment("pii", Some("b"))], Ok("Second"))]
#[case::two_listed(vec![deployment("pii", Some("a")), deployment("toxic", Some("b")), deployment("pii", Some("a"))], Ok("First"))]
#[case::no_id(vec![deployment("pii", None)], Err("No callback found for guardrail: pii"))]
#[case::unknown_id(vec![deployment("pii", Some("zzz"))], Err("No callback found for guardrail: pii"))]
#[case::unlisted(vec![deployment("toxic", Some("a"))], Err("No guardrail found with name: pii"))]
fn the_balancer_finds_the_picked_callback_by_its_id(
    #[case] guardrails: Vec<RouterGuardrail>,
    #[case] expected: Result<&str, &str>,
) {
    let harness = harness();
    let router = balancer(guardrails, &harness);
    let picked = router
        .available_guardrail(NAME)
        .map(|callback| callback.class_name().to_owned())
        .map_err(|error| error.to_string());
    assert_eq!(picked, expected.map(str::to_owned).map_err(str::to_owned));
}

#[test]
fn a_merge_keeps_every_list_entry_and_skips_fields_the_run_did_not_change() {
    let shared = map(r#"{"metadata": {"keep": 1, "list": ["a"]}}"#);
    let mut live = map(
        r#"{"metadata": {"keep": 2, "list": ["a", "b"], "old": "x"}, "litellm_metadata": "text"}"#,
    );
    let run = map(
        r#"{"model": "edited", "metadata": {"keep": 1, "list": ["a", "c"], "old": "y"}, "litellm_metadata": {"flag": true}, "other": {"a": 1}}"#,
    );
    merge_metadata(&mut live, &shared, &run);
    let expected = map(
        r#"{"metadata": {"keep": 2, "list": ["a", "b", "c"], "old": "y"}, "litellm_metadata": {"flag": true}}"#,
    );
    assert_eq!(live, expected);
}

#[test]
fn a_merge_creates_the_bucket_the_live_request_lacks() {
    let shared = map("{}");
    let mut live = map("{}");
    let run = map(r#"{"metadata": {"list": ["a"]}, "litellm_metadata": 3}"#);
    merge_metadata(&mut live, &shared, &run);
    assert_eq!(live, map(r#"{"metadata": {"list": ["a"]}}"#));
}
