#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, Mode, ModeTarget};
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_base::settings::GuardrailSettings;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::GuardrailSkipFlags;
use betterlite_guardrails::guardrail_records::record::GuardrailStatus;
use betterlite_guardrails::unified_dispatch::registry::ResponseKind;
use rstest::rstest;

use crate::guardrails::unified_guardrail::hooks::{
    CallResponse, GuardrailSkipDefaults, HookRequest,
};
use crate::guardrails::unified_guardrail::recorder::{
    AppliedGuardrailRecorder as _, MetadataGuardrailRecorder,
};
use crate::guardrails::unified_guardrail::tests::support::{
    FakeGuardrail, Hooks, NAME, SpanLog, hooks, object, principal, settings, to_run, value,
};

type Data = BTreeMap<String, GuardrailValue>;

const PROMPT: &str = r#"{"model": "m", "prompt": "secret"}"#;

const TEXT_RESPONSE: &str =
    r#"{"object": "text_completion", "choices": [{"index": 0, "text": "secret"}]}"#;

fn request<'call>(
    data: &'call mut Data,
    user: &'call Data,
    principal: &'call Principal,
) -> HookRequest<'call> {
    HookRequest {
        data,
        user_api_key_dict: user,
        principal,
        logging_obj: None,
        logging_call_type: None,
    }
}

fn plain_hooks() -> (Hooks, Arc<SpanLog>) {
    let spans = Arc::new(SpanLog::default());
    (hooks(&spans, GuardrailSkipDefaults::default()), spans)
}

fn hooked(event: &str) -> GuardrailSettings {
    let mut chosen = settings();
    chosen.default_on = true;
    chosen.event_hook = Some(LitellmParamsMode::One(event.to_owned()));
    chosen
}

fn self_recording() -> GuardrailSettings {
    let mut chosen = settings();
    chosen.switches.records_own_information = true;
    chosen
}

fn recorded(hooks: &Hooks) -> Vec<Option<String>> {
    hooks.recorder.0.lock().unwrap().clone()
}

fn statuses(spans: &SpanLog) -> Vec<GuardrailStatus> {
    spans
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|record| record.guardrail_status)
        .collect()
}

fn modify(original_response: Option<GuardrailValue>) -> GuardrailFailure {
    GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
        message: String::from("blocked"),
        model: String::from("m"),
        request_data: BTreeMap::new(),
        guardrail_name: Some(NAME.to_owned()),
        detection_info: BTreeMap::new(),
        original_response,
    }))
}

fn blocked() -> GuardrailFailure {
    GuardrailFailure::Http {
        status: 400,
        detail: String::from("blocked"),
    }
}

#[tokio::test]
async fn input_without_a_guardrail_leaves_the_data() {
    let (hooks, spans) = plain_hooks();
    let (mut data, user, caller) = (object(PROMPT), object(r#"{"user_id": "u1"}"#), principal());
    hooks
        .pre_call(
            None,
            request(&mut data, &user, &caller),
            CallType::TextCompletion,
        )
        .await
        .unwrap();
    hooks
        .moderation(
            None,
            request(&mut data, &user, &caller),
            CallType::TextCompletion,
        )
        .await
        .unwrap();
    assert_eq!(data, object(PROMPT));
    assert!(statuses(&spans).is_empty());
    assert!(recorded(&hooks).is_empty());
}

#[rstest]
#[case::pre_call_refused(true, "post_call", CallType::TextCompletion)]
#[case::moderation_refused(false, "pre_call", CallType::TextCompletion)]
#[case::pre_call_unmapped(true, "pre_call", CallType::Ocr)]
#[case::moderation_unmapped(false, "during_call", CallType::Aocr)]
#[tokio::test]
async fn input_leaves_the_data_when_nothing_runs(
    #[case] pre_call: bool,
    #[case] event: &str,
    #[case] call_type: CallType,
) {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let run = to_run(&guardrail, hooked(event));
    let (mut data, user, caller) = (object(PROMPT), object(r#"{"user_id": "u1"}"#), principal());
    let call = request(&mut data, &user, &caller);
    if pre_call {
        hooks.pre_call(Some(&run), call, call_type).await.unwrap();
    } else {
        hooks.moderation(Some(&run), call, call_type).await.unwrap();
    }
    assert_eq!(data, object(PROMPT));
    assert!(guardrail.seen().is_empty());
    assert!(recorded(&hooks).is_empty());
}

#[rstest]
#[case::pre_call(true)]
#[case::moderation(false)]
#[tokio::test]
async fn input_rewrites_and_adds_the_key_metadata(#[case] pre_call: bool) {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let run = to_run(&guardrail, settings());
    let (mut data, user, caller) = (object(PROMPT), object(r#"{"user_id": "u1"}"#), principal());
    let call = request(&mut data, &user, &caller);
    if pre_call {
        hooks
            .pre_call(Some(&run), call, CallType::TextCompletion)
            .await
            .unwrap();
    } else {
        hooks
            .moderation(Some(&run), call, CallType::TextCompletion)
            .await
            .unwrap();
    }
    assert_eq!(data.get("prompt"), Some(&value(r#""[masked]""#)));
    let Some(GuardrailValue::Object(metadata)) = data.get("litellm_metadata") else {
        panic!("no litellm_metadata in {data:?}");
    };
    assert_eq!(
        metadata.get("user_api_key_user_id"),
        Some(&value(r#""u1""#))
    );
}

#[tokio::test]
async fn input_keeps_litellm_metadata_the_request_already_has() {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let run = to_run(&guardrail, settings());
    let mut data = object(r#"{"prompt": "secret", "litellm_metadata": {"trace": "t1"}}"#);
    let (user, caller) = (object(r#"{"user_id": "u1"}"#), principal());
    hooks
        .pre_call(
            Some(&run),
            request(&mut data, &user, &caller),
            CallType::TextCompletion,
        )
        .await
        .unwrap();
    let Some(GuardrailValue::Object(metadata)) = data.get("litellm_metadata") else {
        panic!("no litellm_metadata in {data:?}");
    };
    assert_eq!(metadata.get("trace"), Some(&value(r#""t1""#)));
    assert!(!metadata.contains_key("user_api_key_user_id"));
}

#[rstest]
#[case::pre_mcp_call_on_mcp(true, "pre_mcp_call", CallType::CallMcpTool, true)]
#[case::pre_mcp_call_on_completion(true, "pre_mcp_call", CallType::TextCompletion, false)]
#[case::pre_call_on_mcp(true, "pre_call", CallType::CallMcpTool, false)]
#[case::pre_call_on_completion(true, "pre_call", CallType::TextCompletion, true)]
#[case::during_mcp_call_on_mcp(false, "during_mcp_call", CallType::ListMcpTools, true)]
#[case::during_call_on_mcp(false, "during_call", CallType::CallMcpTool, false)]
#[case::during_call_on_completion(false, "during_call", CallType::TextCompletion, true)]
#[tokio::test]
async fn input_mcp_call_types_use_the_mcp_events(
    #[case] pre_call: bool,
    #[case] event: &str,
    #[case] call_type: CallType,
    #[case] runs: bool,
) {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let run = to_run(&guardrail, hooked(event));
    let mut data = object(
        r#"{"prompt": "secret", "mcp_tool_name": "lookup", "mcp_arguments": {"q": "secret"}}"#,
    );
    let (user, caller) = (Data::new(), principal());
    let call = request(&mut data, &user, &caller);
    if pre_call {
        hooks.pre_call(Some(&run), call, call_type).await.unwrap();
    } else {
        hooks.moderation(Some(&run), call, call_type).await.unwrap();
    }
    assert_eq!(!guardrail.seen().is_empty(), runs);
}

#[tokio::test]
async fn input_gate_error_is_returned() {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let mut chosen = settings();
    chosen.default_on = true;
    chosen.event_hook = Some(LitellmParamsMode::Tagged(Mode {
        tags: BTreeMap::from([(
            String::from("team"),
            ModeTarget::One(String::from("pre_call")),
        )]),
        default: None,
    }));
    let run = to_run(&guardrail, chosen);
    let (mut data, user, caller) = (object(PROMPT), Data::new(), principal());
    let failure = hooks
        .pre_call(
            Some(&run),
            request(&mut data, &user, &caller),
            CallType::TextCompletion,
        )
        .await
        .unwrap_err();
    assert!(matches!(failure, GuardrailFailure::Other(_)), "{failure:?}");
    assert!(guardrail.seen().is_empty());
}

#[rstest]
#[case::records(settings(), vec![Some(NAME.to_owned())])]
#[case::self_recording_skips(self_recording(), vec![])]
#[tokio::test]
async fn recorder_pre_call_records_the_name_unless_self_recording(
    #[case] chosen: GuardrailSettings,
    #[case] expected: Vec<Option<String>>,
) {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let run = to_run(&guardrail, chosen);
    let (mut data, user, caller) = (object(PROMPT), Data::new(), principal());
    hooks
        .pre_call(
            Some(&run),
            request(&mut data, &user, &caller),
            CallType::TextCompletion,
        )
        .await
        .unwrap();
    assert_eq!(recorded(&hooks), expected);
}

#[tokio::test]
async fn recorder_moderation_never_records() {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let run = to_run(&guardrail, settings());
    let (mut data, user, caller) = (object(PROMPT), Data::new(), principal());
    hooks
        .moderation(
            Some(&run),
            request(&mut data, &user, &caller),
            CallType::TextCompletion,
        )
        .await
        .unwrap();
    assert_eq!(guardrail.seen().len(), 1);
    assert!(recorded(&hooks).is_empty());
}

#[rstest]
#[case::records(settings(), vec![Some(NAME.to_owned())])]
#[case::self_recording_skips(self_recording(), vec![])]
#[tokio::test]
async fn recorder_post_call_records_the_name_unless_self_recording(
    #[case] chosen: GuardrailSettings,
    #[case] expected: Vec<Option<String>>,
) {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let run = to_run(&guardrail, chosen);
    let (mut data, user, caller) = (object(PROMPT), Data::new(), principal());
    let response = CallResponse {
        body: value(TEXT_RESPONSE),
        kind: ResponseKind::TextCompletion,
    };
    hooks
        .post_call_success(Some(&run), request(&mut data, &user, &caller), response)
        .await
        .unwrap();
    assert_eq!(recorded(&hooks), expected);
}

#[rstest]
#[case::no_name("{}", None, "{}")]
#[case::creates_the_list("{}", Some("a"), r#"{"metadata": {"applied_guardrails": ["a"]}}"#)]
#[case::appends(
    r#"{"metadata": {"applied_guardrails": ["b"]}}"#,
    Some("a"),
    r#"{"metadata": {"applied_guardrails": ["b", "a"]}}"#
)]
#[case::adds_once(
    r#"{"metadata": {"applied_guardrails": ["a"]}}"#,
    Some("a"),
    r#"{"metadata": {"applied_guardrails": ["a"]}}"#
)]
#[case::litellm_bucket(
    r#"{"litellm_metadata": {"x": 1}}"#,
    Some("a"),
    r#"{"litellm_metadata": {"x": 1, "applied_guardrails": ["a"]}}"#
)]
#[case::bucket_not_an_object(
    r#"{"metadata": "x"}"#,
    Some("a"),
    r#"{"metadata": {"applied_guardrails": ["a"]}}"#
)]
#[case::list_not_a_list(
    r#"{"metadata": {"applied_guardrails": "b"}}"#,
    Some("a"),
    r#"{"metadata": {"applied_guardrails": ["a"]}}"#
)]
fn recorder_metadata_adds_each_name_once(
    #[case] before: &str,
    #[case] name: Option<&str>,
    #[case] after: &str,
) {
    let mut data = object(before);
    MetadataGuardrailRecorder.record(&mut data, name);
    assert_eq!(data, object(after));
}

#[rstest]
#[case::pre_call(0_u8)]
#[case::moderation(1_u8)]
#[case::post_call(2_u8)]
#[tokio::test]
async fn apply_record_writes_one_success_per_call(#[case] hook: u8) {
    let (hooks, spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let run = to_run(&guardrail, settings());
    let (mut data, user, caller) = (object(PROMPT), Data::new(), principal());
    let call = request(&mut data, &user, &caller);
    match hook {
        0 => hooks
            .pre_call(Some(&run), call, CallType::TextCompletion)
            .await
            .unwrap(),
        1 => hooks
            .moderation(Some(&run), call, CallType::TextCompletion)
            .await
            .unwrap(),
        _ => {
            let response = CallResponse {
                body: value(TEXT_RESPONSE),
                kind: ResponseKind::TextCompletion,
            };
            hooks
                .post_call_success(Some(&run), call, response)
                .await
                .unwrap();
        }
    }
    assert_eq!(guardrail.seen().len(), 1);
    assert_eq!(statuses(&spans), vec![GuardrailStatus::Success]);
}

#[tokio::test]
async fn apply_record_failure_records_the_intervention_and_returns_it() {
    let (hooks, spans) = plain_hooks();
    let guardrail = FakeGuardrail::failing(blocked());
    let run = to_run(&guardrail, settings());
    let (mut data, user, caller) = (object(PROMPT), Data::new(), principal());
    let failure = hooks
        .pre_call(
            Some(&run),
            request(&mut data, &user, &caller),
            CallType::TextCompletion,
        )
        .await
        .unwrap_err();
    assert_eq!(failure, blocked());
    assert_eq!(statuses(&spans), vec![GuardrailStatus::GuardrailIntervened]);
    assert!(recorded(&hooks).is_empty());
}

#[tokio::test]
async fn apply_record_self_recording_guardrail_writes_no_entry() {
    let (hooks, spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let run = to_run(&guardrail, self_recording());
    let (mut data, user, caller) = (object(PROMPT), Data::new(), principal());
    hooks
        .pre_call(
            Some(&run),
            request(&mut data, &user, &caller),
            CallType::TextCompletion,
        )
        .await
        .unwrap();
    assert_eq!(guardrail.seen().len(), 1);
    assert!(statuses(&spans).is_empty());
}

#[rstest]
#[case::no_guardrail(false, settings())]
#[case::gate_refused(true, hooked("pre_call"))]
#[tokio::test]
async fn post_call_returns_none_when_nothing_runs(
    #[case] with_guardrail: bool,
    #[case] chosen: GuardrailSettings,
) {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let run = to_run(&guardrail, chosen);
    let (mut data, user, caller) = (object(PROMPT), Data::new(), principal());
    let response = CallResponse {
        body: value(TEXT_RESPONSE),
        kind: ResponseKind::TextCompletion,
    };
    let chosen_guardrail = with_guardrail.then_some(&run);
    let answer = hooks
        .post_call_success(
            chosen_guardrail,
            request(&mut data, &user, &caller),
            response,
        )
        .await
        .unwrap();
    assert_eq!(answer, None);
    assert!(guardrail.seen().is_empty());
}

#[rstest]
#[case::route_wins(Some("/v1/completions"), ResponseKind::ChatCompletion, None, true)]
#[case::route_without_call_types(Some("/not-a-route"), ResponseKind::TextCompletion, None, true)]
#[case::kind_fallback(None, ResponseKind::TextCompletion, None, true)]
#[case::route_over_logged(
    Some("/v1/completions"),
    ResponseKind::Other,
    Some(CallType::Ocr),
    true
)]
#[case::logged_pass_through(None, ResponseKind::Other, Some(CallType::PassThroughEndpoint), true)]
#[case::logged_completion_ignored(None, ResponseKind::Other, Some(CallType::TextCompletion), false)]
#[case::logged_ocr_unmapped(None, ResponseKind::Other, Some(CallType::Ocr), false)]
#[case::unresolved(None, ResponseKind::Other, None, false)]
#[tokio::test]
async fn post_call_resolves_route_then_kind_then_logged_call_type(
    #[case] route: Option<&str>,
    #[case] kind: ResponseKind,
    #[case] logged: Option<CallType>,
    #[case] scanned: bool,
) {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let run = to_run(&guardrail, settings());
    let mut user = Data::new();
    if let Some(path) = route {
        user.insert(
            String::from("request_route"),
            GuardrailValue::Text(path.to_owned()),
        );
    }
    let (mut data, caller) = (object(PROMPT), principal());
    let mut call = request(&mut data, &user, &caller);
    call.logging_call_type = logged;
    let response = CallResponse {
        body: value(TEXT_RESPONSE),
        kind,
    };
    let answer = hooks
        .post_call_success(Some(&run), call, response)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(!guardrail.seen().is_empty(), scanned);
    if !scanned {
        assert_eq!(answer, value(TEXT_RESPONSE));
    }
}

#[tokio::test]
async fn post_call_rewrites_through_the_text_completion_adapter() {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::masking();
    let run = to_run(&guardrail, settings());
    let (mut data, user, caller) = (object(PROMPT), Data::new(), principal());
    let response = CallResponse {
        body: value(TEXT_RESPONSE),
        kind: ResponseKind::TextCompletion,
    };
    let answer = hooks
        .post_call_success(Some(&run), request(&mut data, &user, &caller), response)
        .await
        .unwrap();
    assert_eq!(
        answer,
        Some(value(
            r#"{"object": "text_completion", "choices": [{"index": 0, "text": "[masked]"}]}"#
        ))
    );
}

#[rstest]
#[case::fills_a_missing_original(None, value(TEXT_RESPONSE))]
#[case::keeps_a_given_original(Some(value(r#""earlier""#)), value(r#""earlier""#))]
#[tokio::test]
async fn post_call_block_carries_the_original_response(
    #[case] given: Option<GuardrailValue>,
    #[case] expected: GuardrailValue,
) {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::failing(modify(given));
    let run = to_run(&guardrail, settings());
    let (mut data, user, caller) = (object(PROMPT), Data::new(), principal());
    let response = CallResponse {
        body: value(TEXT_RESPONSE),
        kind: ResponseKind::TextCompletion,
    };
    let failure = hooks
        .post_call_success(Some(&run), request(&mut data, &user, &caller), response)
        .await
        .unwrap_err();
    let GuardrailFailure::ModifyResponse(details) = failure else {
        panic!("expected a ModifyResponse failure, got {failure:?}");
    };
    assert_eq!(details.original_response, Some(expected));
    assert!(recorded(&hooks).is_empty());
}

#[tokio::test]
async fn post_call_other_failures_pass_through() {
    let (hooks, _spans) = plain_hooks();
    let guardrail = FakeGuardrail::failing(blocked());
    let run = to_run(&guardrail, settings());
    let (mut data, user, caller) = (object(PROMPT), Data::new(), principal());
    let response = CallResponse {
        body: value(TEXT_RESPONSE),
        kind: ResponseKind::TextCompletion,
    };
    let failure = hooks
        .post_call_success(Some(&run), request(&mut data, &user, &caller), response)
        .await
        .unwrap_err();
    assert_eq!(failure, blocked());
}

#[rstest]
#[case::default_used_when_unset(None, true, false)]
#[case::own_false_beats_default_true(Some(false), true, true)]
#[case::own_true_beats_default_false(Some(true), false, false)]
#[case::scanned_by_default(None, false, true)]
#[tokio::test]
async fn skip_system_default_applies_only_when_the_guardrail_leaves_it_unset(
    #[case] own: Option<bool>,
    #[case] default: bool,
    #[case] system_scanned: bool,
) {
    let spans = Arc::new(SpanLog::default());
    let hooks = hooks(
        &spans,
        GuardrailSkipDefaults {
            system: default,
            tool: false,
        },
    );
    let guardrail = FakeGuardrail::masking();
    let mut run = to_run(&guardrail, settings());
    run.skip_flags = GuardrailSkipFlags {
        skip_system_message: own,
        skip_tool_message: None,
        scan_only_tool_results: None,
    };
    let mut data = object(
        r#"{"model": "m", "messages": [{"role": "system", "content": "system-text"}, {"role": "user", "content": "user-text"}]}"#,
    );
    let (user, caller) = (Data::new(), principal());
    hooks
        .pre_call(
            Some(&run),
            request(&mut data, &user, &caller),
            CallType::Completion,
        )
        .await
        .unwrap();
    let sent: Vec<String> = guardrail.seen_texts().into_iter().flatten().collect();
    assert!(sent.contains(&String::from("user-text")), "{sent:?}");
    assert_eq!(
        sent.contains(&String::from("system-text")),
        system_scanned,
        "{sent:?}"
    );
}

#[rstest]
#[case::default_used_when_unset(None, true, false)]
#[case::own_false_beats_default_true(Some(false), true, true)]
#[case::scanned_by_default(None, false, true)]
#[tokio::test]
async fn skip_tool_default_reaches_the_moderation_hook(
    #[case] own: Option<bool>,
    #[case] default: bool,
    #[case] tool_scanned: bool,
) {
    let spans = Arc::new(SpanLog::default());
    let hooks = hooks(
        &spans,
        GuardrailSkipDefaults {
            system: false,
            tool: default,
        },
    );
    let guardrail = FakeGuardrail::masking();
    let mut run = to_run(&guardrail, settings());
    run.skip_flags = GuardrailSkipFlags {
        skip_system_message: None,
        skip_tool_message: own,
        scan_only_tool_results: None,
    };
    let mut data = object(
        r#"{"model": "m", "messages": [{"role": "user", "content": "user-text"}, {"role": "tool", "tool_call_id": "c1", "content": "tool-text"}]}"#,
    );
    let (user, caller) = (Data::new(), principal());
    hooks
        .moderation(
            Some(&run),
            request(&mut data, &user, &caller),
            CallType::Completion,
        )
        .await
        .unwrap();
    let sent: Vec<String> = guardrail.seen_texts().into_iter().flatten().collect();
    assert!(sent.contains(&String::from("user-text")), "{sent:?}");
    assert_eq!(
        sent.contains(&String::from("tool-text")),
        tool_scanned,
        "{sent:?}"
    );
}
