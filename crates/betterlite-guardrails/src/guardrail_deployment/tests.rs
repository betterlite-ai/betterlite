#![cfg(test)]

mod fixtures;

use rstest::rstest;
use std::collections::BTreeSet;
use uuid::Uuid;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_core::routes::CallType;

use super::hook_target::{HookTarget, PreCallOutcome, StampedKeyFields, stamped_key_fields};
use super::hooks::{DeploymentHookError, PreCallContext};
use super::logging_hook::{LoggedCall, LoggedResultKind, LoggingHookError};
use super::markers::PreCallToken;
use super::messages_hook::MessagesHookError;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use fixtures::{
    GIVEN, GUARDED, HOOKED, LOGGED, PostCallSeen, PreCallSeen, Recorder, SCAN_REQUEST, Scanner,
    VERDICT, applying, boom, capture, data, given, guardrail, key, legacy_call, pre_call_guardrail,
    pre_completion, record, recording_registry, registry, rejected, reroute, run_pre, scan_logged,
    shared, tag_mode_error, tagged, token, two_translations, value, with_hook,
};

#[test]
fn marker_token_from_a_uuid_is_thirty_two_lowercase_hex_characters() {
    let made = PreCallToken::from(Uuid::from_u128(0xab));
    assert_eq!(made.0, "000000000000000000000000000000ab");
}

#[rstest]
#[case::named(Some("g"), Some("tok:g"))]
#[case::unnamed(None, None)]
#[case::empty_name(Some(""), Some("tok:"))]
fn marker_text_joins_the_token_and_the_name(
    #[case] name: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let made = guardrail(name).pre_call_marker(&token());
    assert_eq!(made.as_deref(), expected);
}

#[rstest]
#[case::appends(
    r#"{"metadata":{"_pre_call_executed_guardrails":["other"]}}"#,
    r#"{"metadata":{"_pre_call_executed_guardrails":["other","tok:g"]}}"#
)]
#[case::does_not_repeat(
    r#"{"metadata":{"_pre_call_executed_guardrails":["tok:g"]}}"#,
    r#"{"metadata":{"_pre_call_executed_guardrails":["tok:g"]}}"#
)]
#[case::replaces_a_text(
    r#"{"metadata":{"_pre_call_executed_guardrails":"x"}}"#,
    r#"{"metadata":{"_pre_call_executed_guardrails":["tok:g"]}}"#
)]
#[case::replaces_a_null(
    r#"{"metadata":{"_pre_call_executed_guardrails":null}}"#,
    r#"{"metadata":{"_pre_call_executed_guardrails":["tok:g"]}}"#
)]
#[case::keeps_other_keys(
    r#"{"metadata":{"a":1}}"#,
    r#"{"metadata":{"a":1,"_pre_call_executed_guardrails":["tok:g"]}}"#
)]
#[case::first_object_bucket_only(
    r#"{"metadata":{},"litellm_metadata":{}}"#,
    r#"{"metadata":{"_pre_call_executed_guardrails":["tok:g"]},"litellm_metadata":{}}"#
)]
#[case::second_bucket_when_the_first_is_no_object(
    r#"{"metadata":"x","litellm_metadata":{}}"#,
    r#"{"metadata":"x","litellm_metadata":{"_pre_call_executed_guardrails":["tok:g"]}}"#
)]
#[case::creates_metadata(
    r#"{"model":"m"}"#,
    r#"{"model":"m","metadata":{"_pre_call_executed_guardrails":["tok:g"]}}"#
)]
#[case::replaces_a_null_metadata(
    r#"{"metadata":null}"#,
    r#"{"metadata":{"_pre_call_executed_guardrails":["tok:g"]}}"#
)]
fn marker_mark_records_the_marker_in_the_first_object_bucket(
    #[case] before: &str,
    #[case] after: &str,
) {
    let mut request = data(before);
    guardrail(Some("g")).mark_pre_call_hook_ran(&token(), &mut request);
    assert_eq!(request, data(after));
}

#[test]
fn marker_mark_does_nothing_for_an_unnamed_guardrail() {
    let mut request = data(r#"{"metadata":{}}"#);
    guardrail(None).mark_pre_call_hook_ran(&token(), &mut request);
    assert_eq!(request, data(r#"{"metadata":{}}"#));
}

#[rstest]
#[case::metadata(r#"{"metadata":{"_pre_call_executed_guardrails":["tok:g"]}}"#, true)]
#[case::litellm_metadata(
    r#"{"litellm_metadata":{"_pre_call_executed_guardrails":["a","tok:g"]}}"#,
    true
)]
#[case::second_bucket_when_the_first_has_none(
    r#"{"metadata":{},"litellm_metadata":{"_pre_call_executed_guardrails":["tok:g"]}}"#,
    true
)]
#[case::other_token(r#"{"metadata":{"_pre_call_executed_guardrails":["old:g"]}}"#, false)]
#[case::other_name(r#"{"metadata":{"_pre_call_executed_guardrails":["tok:h"]}}"#, false)]
#[case::not_a_list(r#"{"metadata":{"_pre_call_executed_guardrails":"tok:g"}}"#, false)]
#[case::no_key(r#"{"metadata":{}}"#, false)]
#[case::no_bucket(r#"{}"#, false)]
fn marker_already_ran_looks_in_both_buckets(#[case] request: &str, #[case] expected: bool) {
    let ran = guardrail(Some("g")).pre_call_hook_already_ran(&token(), &data(request));
    assert_eq!(ran, expected);
}

#[test]
fn marker_already_ran_is_false_for_an_unnamed_guardrail() {
    let request = data(r#"{"metadata":{"_pre_call_executed_guardrails":["tok:None"]}}"#);
    assert!(!guardrail(None).pre_call_hook_already_ran(&token(), &request));
}

#[test]
fn marker_a_marked_request_reads_as_already_ran() {
    let mut request = data(r#"{"model":"m"}"#);
    let base = guardrail(Some("g"));
    base.mark_pre_call_hook_ran(&token(), &mut request);
    assert!(base.pre_call_hook_already_ran(&token(), &request));
}

#[test]
fn stamped_reads_every_field_from_metadata() {
    let request = data(
        r#"{"metadata":{"user_api_key_user_id":"u","user_api_key_team_id":"t","user_api_key_end_user_id":"e","user_api_key_hash":"h","user_api_key_request_route":"/chat"}}"#,
    );
    let expected = StampedKeyFields {
        user_id: Some("u".to_owned()),
        team_id: Some("t".to_owned()),
        end_user_id: Some("e".to_owned()),
        api_key: Some("h".to_owned()),
        request_route: Some("/chat".to_owned()),
    };
    assert_eq!(stamped_key_fields(&request), expected);
}

#[rstest]
#[case::litellm_metadata_wins(
    r#"{"metadata":{"user_api_key_user_id":"a"},"litellm_metadata":{"user_api_key_user_id":"b"}}"#,
    Some("b")
)]
#[case::a_null_litellm_metadata_hides_metadata(
    r#"{"metadata":{"user_api_key_user_id":"a"},"litellm_metadata":null}"#,
    None
)]
#[case::a_non_text_value(r#"{"metadata":{"user_api_key_user_id":5}}"#, None)]
#[case::no_metadata(r#"{}"#, None)]
#[case::metadata_is_no_object(r#"{"metadata":"x"}"#, None)]
fn stamped_picks_the_bucket_and_only_text_values(
    #[case] request: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(stamped_key_fields(&data(request)), key(expected));
}

#[rstest]
#[case::no_apply_interface(false, false, HookTarget::Own)]
#[case::native_without_apply(false, true, HookTarget::Own)]
#[case::native_hooks(true, true, HookTarget::Own)]
#[case::unified(true, false, HookTarget::Unified)]
fn stamped_hook_target_follows_the_switches(
    #[case] uses_apply: bool,
    #[case] native: bool,
    #[case] expected: HookTarget,
) {
    let mut base = guardrail(Some("g"));
    base.settings.switches.uses_apply_guardrail_interface = uses_apply;
    base.settings.switches.use_native_lifecycle_hooks = native;
    assert_eq!(base.hook_target(), expected);
}

#[tokio::test]
async fn pre_hook_replaces_the_messages_and_hands_the_hook_the_request() {
    let hooks = Recorder::answering(PreCallOutcome::Data(data(HOOKED)));
    let kwargs = data(
        r#"{"guardrails":["g"],"model":"m","messages":[{"role":"user","content":"hi"}],"metadata":{"user_api_key_user_id":"u"}}"#,
    );
    let result = pre_completion(&hooks, kwargs.clone()).await.unwrap();
    assert_eq!(result.get("model"), kwargs.get("model"));
    assert_eq!(
        result.get("messages"),
        data(HOOKED).get("messages"),
        "the hook's messages replace the request's"
    );
    assert_eq!(
        hooks.pre_calls(),
        vec![PreCallSeen {
            target: HookTarget::Own,
            key: key(Some("u")),
            data: kwargs,
            call_type: CallType::Completion,
        }]
    );
}

#[tokio::test]
async fn pre_hook_adds_messages_the_request_did_not_have() {
    let hooks = Recorder::answering(PreCallOutcome::Data(data(HOOKED)));
    let result = run_pre(
        &guardrail(Some("g")),
        &hooks,
        data(r#"{"guardrails":[]}"#),
        Some(CallType::Acompletion),
    )
    .await
    .unwrap();
    assert_eq!(result.get("messages"), data(HOOKED).get("messages"));
}

#[rstest]
#[case::unchanged(PreCallOutcome::Unchanged)]
#[case::text(PreCallOutcome::Text("nope".to_owned()))]
#[case::failure(PreCallOutcome::Failure(Box::new(GuardrailFailure::Other("x".to_owned()))))]
#[case::data_without_messages(PreCallOutcome::Data(data(r#"{"other":1}"#)))]
#[case::null_messages(PreCallOutcome::Data(data(r#"{"messages":null}"#)))]
#[tokio::test]
async fn pre_hook_keeps_the_request_unless_the_hook_returns_messages(
    #[case] outcome: PreCallOutcome,
) {
    let kwargs = data(GUARDED);
    let result = pre_completion(&Recorder::answering(outcome), kwargs.clone()).await;
    assert_eq!(result, Ok(kwargs));
}

#[rstest]
#[case::missing(r#"{"messages":[]}"#)]
#[case::not_a_list(r#"{"guardrails":"g"}"#)]
#[case::null(r#"{"guardrails":null}"#)]
#[tokio::test]
async fn pre_hook_skips_a_request_without_a_guardrails_list(#[case] request: &str) {
    let hooks = Recorder::answering(PreCallOutcome::Data(data(HOOKED)));
    let result = pre_completion(&hooks, data(request)).await;
    assert_eq!(result, Ok(data(request)));
    assert!(hooks.pre_calls().is_empty());
}

#[rstest]
#[case::marked(
    r#"{"guardrails":["g"],"metadata":{"_pre_call_executed_guardrails":["tok:g"]}}"#,
    0
)]
#[case::marked_by_another_token(
    r#"{"guardrails":["g"],"metadata":{"_pre_call_executed_guardrails":["old:g"]}}"#,
    1
)]
#[tokio::test]
async fn pre_hook_skips_a_hook_that_already_ran(#[case] request: &str, #[case] calls: usize) {
    let hooks = Recorder::answering(PreCallOutcome::Unchanged);
    pre_completion(&hooks, data(request)).await.unwrap();
    assert_eq!(hooks.pre_calls().len(), calls);
}

#[tokio::test]
async fn pre_hook_skips_a_suppressed_guardrail() {
    let hooks = Recorder::answering(PreCallOutcome::Data(data(HOOKED)));
    let marker_token = token();
    let suppressed = BTreeSet::from(["g".to_owned()]);
    let context = PreCallContext {
        token: &marker_token,
        suppressed: &suppressed,
    };
    let kwargs = data(GUARDED);
    let result = guardrail(Some("g"))
        .pre_call_deployment_hook(&hooks, &context, kwargs.clone(), Some(CallType::Completion))
        .await
        .unwrap();
    assert_eq!(result, kwargs);
    assert!(hooks.pre_calls().is_empty());
}

#[tokio::test]
async fn pre_hook_skips_a_guardrail_for_another_event() {
    let hooks = Recorder::answering(PreCallOutcome::Data(data(HOOKED)));
    let base = with_hook(guardrail(Some("g")), "post_call");
    let kwargs = data(GUARDED);
    let result = run_pre(&base, &hooks, kwargs.clone(), Some(CallType::Completion))
        .await
        .unwrap();
    assert_eq!(result, kwargs);
    assert!(hooks.pre_calls().is_empty());
}

#[rstest]
#[case::no_call_type(None)]
#[case::embedding(Some(CallType::Embedding))]
#[case::async_embedding(Some(CallType::Aembedding))]
#[case::text_completion(Some(CallType::TextCompletion))]
#[tokio::test]
async fn pre_hook_only_runs_for_chat_completions(#[case] call_type: Option<CallType>) {
    let hooks = Recorder::answering(PreCallOutcome::Data(data(HOOKED)));
    let kwargs = data(GUARDED);
    let result = run_pre(&guardrail(Some("g")), &hooks, kwargs.clone(), call_type)
        .await
        .unwrap();
    assert_eq!(result, kwargs);
    assert!(hooks.pre_calls().is_empty());
}

#[rstest]
#[case::completion(CallType::Completion)]
#[case::acompletion(CallType::Acompletion)]
#[tokio::test]
async fn pre_hook_runs_for_both_completion_call_types(#[case] call_type: CallType) {
    let hooks = Recorder::answering(PreCallOutcome::Unchanged);
    run_pre(
        &guardrail(Some("g")),
        &hooks,
        data(GUARDED),
        Some(call_type),
    )
    .await
    .unwrap();
    let seen: Vec<CallType> = hooks
        .pre_calls()
        .iter()
        .map(|call| call.call_type)
        .collect();
    assert_eq!(seen, [call_type]);
}

#[tokio::test]
async fn pre_hook_hands_a_unified_guardrail_to_the_unified_target() {
    let hooks = Recorder::answering(PreCallOutcome::Unchanged);
    run_pre(
        &applying(Some("g")),
        &hooks,
        data(GUARDED),
        Some(CallType::Completion),
    )
    .await
    .unwrap();
    assert_eq!(hooks.pre_calls()[0].target, HookTarget::Unified);
}

#[tokio::test]
async fn pre_hook_returns_what_the_hook_raised() {
    let hooks = Recorder {
        pre: Err(boom()),
        ..Recorder::answering(PreCallOutcome::Unchanged)
    };
    let result = pre_completion(&hooks, data(GUARDED)).await;
    assert_eq!(result, Err(DeploymentHookError::Hook(Box::new(boom()))));
}

#[tokio::test]
async fn pre_hook_tag_mode_is_an_error_even_without_a_call_type() {
    let hooks = Recorder::answering(PreCallOutcome::Unchanged);
    let result = run_pre(&tagged(), &hooks, data(r#"{"guardrails":["g"]}"#), None).await;
    assert_eq!(result, Err(tag_mode_error()));
    assert!(hooks.pre_calls().is_empty());
}

#[tokio::test]
async fn pre_hook_checks_the_marker_before_the_gate() {
    let hooks = Recorder::answering(PreCallOutcome::Unchanged);
    let request =
        data(r#"{"guardrails":["g"],"metadata":{"_pre_call_executed_guardrails":["tok:g"]}}"#);
    let result = run_pre(
        &tagged(),
        &hooks,
        request.clone(),
        Some(CallType::Completion),
    )
    .await;
    assert_eq!(result, Ok(request));
}

#[tokio::test]
async fn post_hook_returns_the_hooks_response_and_hands_it_the_request() {
    let replaced = Some(value(r#"{"id":"r2"}"#));
    let hooks = Recorder::post_answering(Ok(replaced.clone()));
    let mut request = data(r#"{"guardrails":["g"],"metadata":{"user_api_key_user_id":"u"}}"#);
    let seen = request.clone();
    let result = guardrail(Some("g"))
        .post_call_deployment_hook(
            &hooks,
            &BTreeSet::new(),
            &mut request,
            value(r#"{"id":"r1"}"#),
        )
        .await
        .unwrap();
    assert_eq!(result, replaced);
    assert_eq!(
        hooks.post_calls(),
        vec![PostCallSeen {
            target: HookTarget::Own,
            key: key(Some("u")),
            data: seen,
            response: value(r#"{"id":"r1"}"#),
        }]
    );
}

#[tokio::test]
async fn post_hook_returns_none_when_the_hook_changes_nothing() {
    let hooks = Recorder::post_answering(Ok(None));
    let mut request = data(r#"{"guardrails":[]}"#);
    let result = applying(Some("g"))
        .post_call_deployment_hook(&hooks, &BTreeSet::new(), &mut request, value("{}"))
        .await
        .unwrap();
    assert_eq!(result, None);
    assert_eq!(hooks.post_calls()[0].target, HookTarget::Unified);
}

#[rstest]
#[case::missing(r#"{}"#)]
#[case::not_a_list(r#"{"guardrails":"g"}"#)]
#[tokio::test]
async fn post_hook_skips_a_request_without_a_guardrails_list(#[case] request: &str) {
    let hooks = Recorder::post_answering(Ok(Some(value("1"))));
    let mut body = data(request);
    let result = tagged()
        .post_call_deployment_hook(&hooks, &BTreeSet::new(), &mut body, value("{}"))
        .await
        .unwrap();
    assert_eq!(result, None);
    assert!(hooks.post_calls().is_empty());
}

#[rstest]
#[case::another_event(with_hook(guardrail(Some("g")), "pre_call"), &[])]
#[case::suppressed(guardrail(Some("g")), &["g"])]
#[tokio::test]
async fn post_hook_skips_a_guardrail_the_gate_turns_off(
    #[case] base: GuardrailBase,
    #[case] suppressed: &[&str],
) {
    let hooks = Recorder::post_answering(Ok(Some(value("1"))));
    let names: BTreeSet<String> = suppressed.iter().map(|name| (*name).to_owned()).collect();
    let mut request = data(r#"{"guardrails":["g"]}"#);
    let result = base
        .post_call_deployment_hook(&hooks, &names, &mut request, value("{}"))
        .await
        .unwrap();
    assert_eq!(result, None);
    assert!(hooks.post_calls().is_empty());
}

#[tokio::test]
async fn post_hook_tag_mode_is_an_error() {
    let hooks = Recorder::post_answering(Ok(None));
    let mut request = data(r#"{"guardrails":["g"]}"#);
    let result = tagged()
        .post_call_deployment_hook(&hooks, &BTreeSet::new(), &mut request, value("{}"))
        .await;
    assert_eq!(result, Err(tag_mode_error()));
}

#[tokio::test]
async fn post_hook_returns_what_the_hook_raised() {
    let hooks = Recorder::post_answering(Err(boom()));
    let mut request = data(r#"{"guardrails":[]}"#);
    let result = guardrail(Some("g"))
        .post_call_deployment_hook(&hooks, &BTreeSet::new(), &mut request, value("{}"))
        .await;
    assert_eq!(result, Err(DeploymentHookError::Hook(Box::new(boom()))));
}

#[tokio::test]
async fn messages_hook_scans_the_given_messages_in_place_of_the_content_keys() {
    let hooks = Recorder::answering(PreCallOutcome::Unchanged);
    let messages = pre_call_guardrail()
        .pre_call_hook_on_messages(&hooks, &data(SCAN_REQUEST), &given())
        .await
        .unwrap();
    assert_eq!(messages, given());
    let expected = data(&format!(
        r#"{{"model":"m","stream":true,"metadata":{{"user_api_key_user_id":"u"}},"messages":{GIVEN}}}"#
    ));
    assert_eq!(
        hooks.pre_calls(),
        vec![PreCallSeen {
            target: HookTarget::Own,
            key: key(Some("u")),
            data: expected,
            call_type: CallType::Acompletion,
        }]
    );
}

#[rstest]
#[case::the_scanned_messages(
    r#"{"messages":[{"role":"tool","content":"clean"}]}"#,
    r#"[{"role":"tool","content":"clean"}]"#
)]
#[case::no_messages(r#"{"other":1}"#, GIVEN)]
#[case::null_messages(r#"{"messages":null}"#, GIVEN)]
#[case::messages_that_are_no_list(r#"{"messages":"x"}"#, GIVEN)]
#[tokio::test]
async fn messages_hook_returns_the_messages_the_hook_answers_with(
    #[case] answer: &str,
    #[case] expected: &str,
) {
    let hooks = Recorder::answering(PreCallOutcome::Data(data(answer)));
    let messages = pre_call_guardrail()
        .pre_call_hook_on_messages(&hooks, &data(SCAN_REQUEST), &given())
        .await
        .unwrap();
    let wanted: Vec<GuardrailValue> = serde_json::from_str(expected).unwrap();
    assert_eq!(messages, wanted);
}

#[tokio::test]
async fn messages_hook_rejects_a_text_answer() {
    let hooks = Recorder::answering(PreCallOutcome::Text("blocked".to_owned()));
    let result = pre_call_guardrail()
        .pre_call_hook_on_messages(&hooks, &data(SCAN_REQUEST), &given())
        .await;
    assert_eq!(result, Err(rejected("blocked")));
}

#[rstest]
#[case::returned(Recorder::answering(PreCallOutcome::Failure(Box::new(boom()))))]
#[case::raised(Recorder { pre: Err(boom()), ..Recorder::answering(PreCallOutcome::Unchanged) })]
#[tokio::test]
async fn messages_hook_fails_with_the_hooks_failure(#[case] hooks: Recorder) {
    let result = pre_call_guardrail()
        .pre_call_hook_on_messages(&hooks, &data(SCAN_REQUEST), &given())
        .await;
    assert_eq!(
        result,
        Err(MessagesHookError::Failed {
            failure: boom(),
            guardrail_name: Some("g".to_owned()),
            event_hook: Some(LitellmParamsMode::One("pre_call".to_owned())),
        })
    );
}

#[rstest]
#[case::the_failures_name(Some("g"), Some("pii"), "pii")]
#[case::this_guardrails_name(Some("g"), None, "g")]
#[case::no_name(None, None, "None")]
#[tokio::test]
async fn messages_hook_blocks_a_reroute_because_retrieved_content_cannot_be_rerouted(
    #[case] own: Option<&str>,
    #[case] raised_by: Option<&str>,
    #[case] shown: &str,
) {
    let hooks = Recorder {
        pre: Err(reroute(raised_by)),
        ..Recorder::answering(PreCallOutcome::Unchanged)
    };
    let result = guardrail(own)
        .pre_call_hook_on_messages(&hooks, &data(SCAN_REQUEST), &given())
        .await;
    let message = format!(
        "{shown} asked to reroute the request to safe-model over retrieved content; a request cannot be rerouted after retrieval, so it was blocked"
    );
    assert_eq!(
        result,
        Err(MessagesHookError::Rejected {
            message,
            guardrail_name: own.map(str::to_owned),
            event_hook: None,
        })
    );
}

#[tokio::test]
async fn messages_hook_does_not_turn_a_returned_reroute_into_a_block() {
    let hooks = Recorder::answering(PreCallOutcome::Failure(Box::new(reroute(Some("pii")))));
    let result = pre_call_guardrail()
        .pre_call_hook_on_messages(&hooks, &data(SCAN_REQUEST), &given())
        .await;
    assert!(matches!(result, Err(MessagesHookError::Failed { .. })));
}

#[rstest]
#[case::unified_copies_metadata(true, r#"{"metadata":{"a":1}}"#, Some(r#"{"a":1}"#))]
#[case::unified_prefers_litellm_metadata(
    true,
    r#"{"metadata":{"a":1},"litellm_metadata":{"b":2}}"#,
    Some(r#"{"b":2}"#)
)]
#[case::unified_ignores_a_non_object_bucket(true, r#"{"metadata":"x"}"#, None)]
#[case::unified_without_a_bucket(true, r#"{}"#, None)]
#[case::own_target_adds_nothing(false, r#"{"metadata":{"a":1}}"#, None)]
#[tokio::test]
async fn messages_hook_carries_the_metadata_bucket_to_a_unified_guardrail(
    #[case] unified: bool,
    #[case] request: &str,
    #[case] expected: Option<&str>,
) {
    let hooks = Recorder::answering(PreCallOutcome::Unchanged);
    let base = if unified {
        applying(Some("g"))
    } else {
        guardrail(Some("g"))
    };
    base.pre_call_hook_on_messages(&hooks, &data(request), &given())
        .await
        .unwrap();
    let scanned = &hooks.pre_calls()[0].data;
    assert_eq!(
        scanned.get("litellm_metadata"),
        expected.map(value).as_ref()
    );
}

#[rstest]
#[case::no_apply_interface(guardrail(Some("g")))]
#[case::another_event(with_hook(applying(Some("g")), "pre_call"))]
#[tokio::test]
async fn logging_hook_skips_a_guardrail_that_does_not_log_this_way(#[case] base: GuardrailBase) {
    let (returned, scans) = scan_logged(&base, LOGGED, "{}", "acompletion").await;
    assert_eq!(returned, data(LOGGED));
    assert!(scans.is_empty());
}

#[rstest]
#[case::logging_only(with_hook(applying(Some("g")), "logging_only"))]
#[case::among_others(applying(Some("g")))]
#[tokio::test]
async fn logging_hook_runs_a_logging_only_guardrail(#[case] base: GuardrailBase) {
    let (_returned, scans) = scan_logged(&base, LOGGED, "{}", "acompletion").await;
    assert!(!scans.is_empty());
}

#[rstest]
#[case::unknown("nonsense")]
#[case::no_translation("embedding")]
#[tokio::test]
async fn logging_hook_skips_a_call_type_without_a_translation_and_says_so(#[case] call_type: &str) {
    let (_guard, logs) = capture();
    let (returned, scans) = scan_logged(&applying(Some("g")), LOGGED, "{}", call_type).await;
    assert_eq!(returned, data(LOGGED));
    assert!(scans.is_empty());
    assert!(logs.text().contains(&format!(
        "Guardrail g: no guardrail translation for call_type={call_type}, skipping logging_only scan"
    )));
}

#[tokio::test]
async fn logging_hook_scans_a_copy_of_the_request_without_the_old_verdict() {
    let result = r#"{"id":"r"}"#;
    let (returned, scans) = scan_logged(&applying(Some("g")), LOGGED, result, "acompletion").await;
    assert_eq!(returned, data(LOGGED));
    let expected = data(
        r#"{"model":"m","litellm_call_id":"c1","messages":[{"role":"user","content":"hi"}],"input":[{"role":"user","content":"hi"}],"tools":[{"type":"function"}],"metadata":{"k":"v"}}"#,
    );
    assert_eq!(scans.len(), 2);
    assert_eq!((scans[0].phase, &scans[0].request), ("input", &expected));
    assert_eq!(scans[1].phase, "output");
    assert_eq!(scans[1].response, Some(value(result)));
}

#[rstest]
#[case::input_when_there_are_no_messages(r#"{"input":"embed me"}"#, r#""embed me""#)]
#[case::input_when_messages_is_null(r#"{"messages":null,"input":"embed me"}"#, r#""embed me""#)]
#[case::messages_before_input(r#"{"messages":"a","input":"b"}"#, r#""a""#)]
#[case::nothing(r#"{}"#, "null")]
#[tokio::test]
async fn logging_hook_scans_the_logged_messages_else_the_logged_input(
    #[case] logged: &str,
    #[case] expected: &str,
) {
    let (_returned, scans) = scan_logged(&applying(Some("g")), logged, "null", "acompletion").await;
    let request = scans[0].request.clone();
    let wanted = value(expected);
    assert_eq!(request.get("messages"), Some(&wanted));
    assert_eq!(request.get("input"), Some(&wanted));
    assert_eq!(request.get("model"), Some(&GuardrailValue::Null));
    assert_eq!(request.get("tools"), Some(&GuardrailValue::Null));
    assert_eq!(request.get("metadata"), Some(&value("{}")));
}

#[rstest]
#[case::async_stream_first(
    r#"{"async_complete_streaming_response":"a","complete_streaming_response":"b"}"#,
    r#""r""#,
    Some(r#""a""#)
)]
#[case::sync_stream_next(r#"{"complete_streaming_response":"b"}"#, r#""r""#, Some(r#""b""#))]
#[case::null_stream_falls_through(
    r#"{"async_complete_streaming_response":null,"complete_streaming_response":"b"}"#,
    r#""r""#,
    Some(r#""b""#)
)]
#[case::the_result(r#"{}"#, r#""r""#, Some(r#""r""#))]
#[case::no_response(r#"{}"#, "null", None)]
#[tokio::test]
async fn logging_hook_scans_the_streamed_response_before_the_result(
    #[case] logged: &str,
    #[case] result: &str,
    #[case] expected: Option<&str>,
) {
    let (_returned, scans) = scan_logged(&applying(Some("g")), logged, result, "acompletion").await;
    let outputs: Vec<Option<GuardrailValue>> = scans
        .iter()
        .filter(|scan| scan.phase == "output")
        .map(|scan| scan.response.clone())
        .collect();
    assert_eq!(
        outputs,
        expected
            .map(|text| Some(value(text)))
            .into_iter()
            .collect::<Vec<_>>()
    );
}

#[rstest]
#[case::model_response(LoggedResultKind::ModelResponse, "{}", "acompletion")]
#[case::other_result(LoggedResultKind::Other, "{}", "completion")]
#[case::assembled_stream(
    LoggedResultKind::Other,
    r#"{"complete_streaming_response":"s"}"#,
    "acompletion"
)]
#[tokio::test]
async fn logging_hook_scans_a_model_response_with_the_acompletion_translation(
    #[case] kind: LoggedResultKind,
    #[case] logged: &str,
    #[case] output_label: &str,
) {
    let seen = shared();
    let result = value(r#""r""#);
    let call = LoggedCall {
        result_kind: kind,
        ..legacy_call(data(logged), &result, "completion")
    };
    applying(Some("g"))
        .logging_hook(&(), &two_translations(&seen), call)
        .await
        .unwrap();
    let labels: Vec<(&str, &str)> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|scan| (scan.phase, scan.label))
        .collect();
    assert_eq!(labels, [("input", "completion"), ("output", output_label)]);
}

#[tokio::test]
async fn logging_hook_a_model_response_needs_an_acompletion_translation() {
    let seen = shared();
    let only_completion = registry(&[(CallType::Completion, Scanner::new("completion", &seen))]);
    let result = value("{}");
    let call = LoggedCall {
        result_kind: LoggedResultKind::ModelResponse,
        ..legacy_call(data(LOGGED), &result, "completion")
    };
    let outcome = applying(Some("g"))
        .logging_hook(&(), &only_completion, call)
        .await;
    assert_eq!(outcome, Err(LoggingHookError::NoOutputTranslation));
    assert!(seen.lock().unwrap().is_empty());
    let plain = legacy_call(data(LOGGED), &result, "completion");
    assert!(
        applying(Some("g"))
            .logging_hook(&(), &only_completion, plain)
            .await
            .is_ok()
    );
}

#[rstest]
#[case::appends_to_the_existing_list(
    r#"{"standard_logging_object":{"guardrail_information":[{"a":1}],"x":2}}"#,
    VERDICT,
    r#"{"standard_logging_object":{"guardrail_information":[{"a":1},{"guardrail_name":"g","guardrail_status":"success"}],"x":2}}"#
)]
#[case::creates_the_list(
    r#"{"standard_logging_object":{"x":2}}"#,
    VERDICT,
    r#"{"standard_logging_object":{"x":2,"guardrail_information":[{"guardrail_name":"g","guardrail_status":"success"}]}}"#
)]
#[case::replaces_a_null_list(
    r#"{"standard_logging_object":{"guardrail_information":null}}"#,
    VERDICT,
    r#"{"standard_logging_object":{"guardrail_information":[{"guardrail_name":"g","guardrail_status":"success"}]}}"#
)]
#[case::appends_every_entry_of_a_list(
    r#"{"standard_logging_object":{"guardrail_information":[0]}}"#,
    r#"[1,2]"#,
    r#"{"standard_logging_object":{"guardrail_information":[0,1,2]}}"#
)]
#[tokio::test]
async fn logging_hook_adds_the_verdict_to_the_standard_logging_object(
    #[case] logged: &str,
    #[case] verdict: &str,
    #[case] expected: &str,
) {
    let returned = record(logged, Some(verdict)).await.unwrap();
    assert_eq!(returned, data(expected));
}

#[rstest]
#[case::no_verdict(r#"{"standard_logging_object":{"x":2}}"#, None)]
#[case::null_verdict(r#"{"standard_logging_object":{"x":2}}"#, Some("null"))]
#[case::empty_list_verdict(r#"{"standard_logging_object":{"x":2}}"#, Some("[]"))]
#[case::no_standard_logging_object(r#"{"model":"m"}"#, Some(VERDICT))]
#[case::standard_logging_object_is_no_object(r#"{"standard_logging_object":"x"}"#, Some(VERDICT))]
#[tokio::test]
async fn logging_hook_leaves_the_request_alone_when_there_is_nothing_to_record(
    #[case] logged: &str,
    #[case] verdict: Option<&str>,
) {
    let returned = record(logged, verdict).await.unwrap();
    assert_eq!(returned, data(logged));
}

#[tokio::test]
async fn logging_hook_rejects_existing_guardrail_information_that_is_no_list() {
    let returned = record(
        r#"{"standard_logging_object":{"guardrail_information":"x"}}"#,
        Some(VERDICT),
    )
    .await;
    assert_eq!(returned, Err(LoggingHookError::ExistingNotList));
}

#[rstest]
#[case::named(Some("g"), "g")]
#[case::unnamed(None, "None")]
#[tokio::test]
async fn logging_hook_logs_a_failed_input_scan_and_still_records(
    #[case] name: Option<&str>,
    #[case] shown: &str,
) {
    let (_guard, logs) = capture();
    let result = value("{}");
    let returned = applying(name)
        .logging_hook(
            &(),
            &recording_registry(Some(VERDICT), true),
            legacy_call(
                data(r#"{"standard_logging_object":{}}"#),
                &result,
                "acompletion",
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        returned,
        data(&format!(
            r#"{{"standard_logging_object":{{"guardrail_information":[{VERDICT}]}}}}"#
        ))
    );
    assert!(logs.text().contains(&format!(
        "Guardrail {shown}: logging_only scan raised: input scan failed"
    )));
}

#[tokio::test]
async fn logging_hook_a_failed_input_scan_skips_the_output_scan() {
    let seen = shared();
    let mut scanner = Scanner::new("acompletion", &seen);
    scanner.fail_input = true;
    let result = value("{}");
    applying(Some("g"))
        .logging_hook(
            &(),
            &registry(&[(CallType::Acompletion, scanner)]),
            legacy_call(data(LOGGED), &result, "acompletion"),
        )
        .await
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn logging_hook_logs_a_failed_output_scan() {
    let (_guard, logs) = capture();
    let seen = shared();
    let mut scanner = Scanner::new("acompletion", &seen);
    scanner.fail_output = true;
    let result = value("{}");
    let kwargs = data(LOGGED);
    let returned = applying(Some("g"))
        .logging_hook(
            &(),
            &registry(&[(CallType::Acompletion, scanner)]),
            legacy_call(kwargs.clone(), &result, "acompletion"),
        )
        .await
        .unwrap();
    assert_eq!(returned, kwargs);
    assert!(
        logs.text()
            .contains("Guardrail g: logging_only scan raised: output scan failed")
    );
}

#[rstest]
#[case::params_no_object(r#"{"litellm_params":"x"}"#, "litellm_params")]
#[case::metadata_no_object(r#"{"litellm_params":{"metadata":[1]}}"#, "litellm_params.metadata")]
#[tokio::test]
async fn logging_hook_rejects_logged_params_that_are_no_objects(
    #[case] logged: &str,
    #[case] field: &'static str,
) {
    let outcome = record(logged, None).await;
    assert_eq!(outcome, Err(LoggingHookError::NotObject { field }));
}

#[rstest]
#[case::null_params(r#"{"litellm_params":null}"#)]
#[case::null_metadata(r#"{"litellm_params":{"metadata":null}}"#)]
#[case::no_metadata(r#"{"litellm_params":{}}"#)]
#[tokio::test]
async fn logging_hook_scans_an_empty_metadata_when_the_logged_one_is_missing(#[case] logged: &str) {
    let (_returned, scans) = scan_logged(&applying(Some("g")), logged, "{}", "acompletion").await;
    assert_eq!(scans[0].request.get("metadata"), Some(&value("{}")));
}

#[tokio::test]
async fn logging_hook_treats_non_object_optional_params_as_a_failed_scan() {
    let (_guard, logs) = capture();
    let logged = r#"{"optional_params":5,"standard_logging_object":{}}"#;
    let (returned, scans) = scan_logged(&applying(Some("g")), logged, "{}", "acompletion").await;
    assert_eq!(returned, data(logged));
    assert!(scans.is_empty());
    assert!(
        logs.text()
            .contains("Guardrail g: logging_only scan raised: optional_params must be an object")
    );
}
