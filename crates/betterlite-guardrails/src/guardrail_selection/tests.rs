#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::error::GuardrailSelectionError;
use super::gate::GuardrailSelection as _;
use super::metadata::{
    admin_metadata, global_guardrails_disabled, opted_out_global_guardrails, requested_guardrails,
};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};

type Data = BTreeMap<String, GuardrailValue>;

fn data(json: &str) -> Data {
    serde_json::from_str(json).unwrap()
}

fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

fn guardrail(name: Option<&str>, hook_json: Option<&str>, default_on: bool) -> GuardrailBase {
    let event_hook: Option<LitellmParamsMode> =
        hook_json.map(|json| serde_json::from_str(json).unwrap());
    GuardrailBase {
        settings: GuardrailSettings {
            guardrail_name: name.map(str::to_owned),
            event_hook,
            default_on,
            supported_event_hooks: None,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: None,
            switches: GuardrailSwitches::default(),
        },
        mode: None,
    }
}

fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn runs(
    base: &GuardrailBase,
    request: &Data,
    event: GuardrailEventHooks,
) -> Result<bool, GuardrailSelectionError> {
    base.should_run_guardrail(request, event, &BTreeSet::new())
}

const PRE: GuardrailEventHooks = GuardrailEventHooks::PreCall;
const POST: GuardrailEventHooks = GuardrailEventHooks::PostCall;
const LOGGING: GuardrailEventHooks = GuardrailEventHooks::LoggingOnly;

#[rstest]
#[case::pre_call(GuardrailEventHooks::PreCall, "pre_call")]
#[case::post_call(GuardrailEventHooks::PostCall, "post_call")]
#[case::during_call(GuardrailEventHooks::DuringCall, "during_call")]
#[case::logging_only(GuardrailEventHooks::LoggingOnly, "logging_only")]
#[case::pre_mcp_call(GuardrailEventHooks::PreMcpCall, "pre_mcp_call")]
#[case::during_mcp_call(GuardrailEventHooks::DuringMcpCall, "during_mcp_call")]
#[case::post_mcp_call(GuardrailEventHooks::PostMcpCall, "post_mcp_call")]
#[case::realtime(
    GuardrailEventHooks::RealtimeInputTranscription,
    "realtime_input_transcription"
)]
fn event_hook_text_matches_each_event_by_its_wire_name(
    #[case] event: GuardrailEventHooks,
    #[case] wire: &str,
) {
    let hook = format!("\"{wire}\"");
    let base = guardrail(Some("g"), Some(&hook), false);
    assert!(base.event_hook_is_event_type(event));
    let other = guardrail(Some("g"), Some("\"unrelated\""), false);
    assert!(!other.event_hook_is_event_type(event));
    let listed = guardrail(Some("g"), Some(&format!("[\"x\", \"{wire}\"]")), false);
    assert!(listed.event_hook_is_event_type(event));
}

#[rstest]
#[case::no_hook(None, PRE, true)]
#[case::no_hook_other_event(None, LOGGING, true)]
#[case::single_match(Some(r#""pre_call""#), PRE, true)]
#[case::single_other(Some(r#""pre_call""#), POST, false)]
#[case::list_member(Some(r#"["pre_call", "post_call"]"#), POST, true)]
#[case::list_other(Some(r#"["pre_call", "post_call"]"#), LOGGING, false)]
#[case::empty_list(Some("[]"), PRE, false)]
#[case::tag_single(Some(r#"{"tags": {"a": "pre_call"}}"#), PRE, true)]
#[case::tag_single_other(Some(r#"{"tags": {"a": "pre_call"}}"#), POST, false)]
#[case::tag_list(Some(r#"{"tags": {"a": ["pre_call", "post_call"]}}"#), POST, true)]
#[case::tag_second_key(Some(r#"{"tags": {"a": "pre_call", "b": ["post_call"]}}"#), POST, true)]
#[case::tag_list_other(Some(r#"{"tags": {"a": ["pre_call"]}}"#), LOGGING, false)]
#[case::default_single(
    Some(r#"{"tags": {"a": "pre_call"}, "default": "post_call"}"#),
    POST,
    true
)]
#[case::default_list(
    Some(r#"{"tags": {"a": "pre_call"}, "default": ["post_call", "logging_only"]}"#),
    LOGGING,
    true
)]
#[case::default_other(
    Some(r#"{"tags": {"a": "pre_call"}, "default": ["post_call"]}"#),
    GuardrailEventHooks::DuringCall,
    false
)]
#[case::no_tags_default_only(Some(r#"{"tags": {}, "default": "pre_call"}"#), PRE, true)]
#[case::no_tags_no_default(Some(r#"{"tags": {}}"#), PRE, false)]
fn event_hook_matches_every_hook_form(
    #[case] hook: Option<&str>,
    #[case] event: GuardrailEventHooks,
    #[case] expected: bool,
) {
    assert_eq!(
        guardrail(Some("g"), hook, false).event_hook_is_event_type(event),
        expected
    );
}

#[rstest]
#[case::root_empty_hides_metadata(r#"{"guardrails": [], "metadata": {"guardrails": ["a"]}}"#, "[]")]
#[case::root_wins(
    r#"{"guardrails": [""], "metadata": {"guardrails": ["a"]}, "litellm_metadata": {"guardrails": ["b"]}}"#,
    r#"[""]"#
)]
#[case::metadata_first(
    r#"{"metadata": {"guardrails": ["a"]}, "litellm_metadata": {"guardrails": ["b"]}}"#,
    r#"["a"]"#
)]
#[case::metadata_empty_hides_litellm(
    r#"{"metadata": {"guardrails": []}, "litellm_metadata": {"guardrails": ["b"]}}"#,
    "[]"
)]
#[case::metadata_null_hides_litellm(
    r#"{"metadata": {"guardrails": null}, "litellm_metadata": {"guardrails": ["b"]}}"#,
    "[]"
)]
#[case::metadata_without_key_hides_nothing(
    r#"{"metadata": {"other": 1}, "litellm_metadata": {"guardrails": ["b"]}}"#,
    r#"["b"]"#
)]
#[case::metadata_not_object_hides_nothing(
    r#"{"metadata": "text", "litellm_metadata": {"guardrails": ["b"]}}"#,
    r#"["b"]"#
)]
#[case::metadata_null_hides_nothing(
    r#"{"metadata": null, "litellm_metadata": {"guardrails": ["b"]}}"#,
    r#"["b"]"#
)]
#[case::only_litellm_metadata(r#"{"litellm_metadata": {"guardrails": ["b"]}}"#, r#"["b"]"#)]
#[case::root_null(r#"{"guardrails": null, "metadata": {"guardrails": ["a"]}}"#, "[]")]
#[case::nowhere(r#"{"model": "m"}"#, "[]")]
fn requested_lookup_order(#[case] request_json: &str, #[case] expected_json: &str) {
    let request = data(request_json);
    let expected: Vec<GuardrailValue> = serde_json::from_str(expected_json).unwrap();
    assert_eq!(requested_guardrails(&request).unwrap(), expected.as_slice());
}

#[rstest]
#[case::root_text(r#"{"guardrails": "a"}"#)]
#[case::root_object(r#"{"guardrails": {"a": 1}}"#)]
#[case::root_bool(r#"{"guardrails": false}"#)]
#[case::metadata_text(r#"{"metadata": {"guardrails": "a"}}"#)]
#[case::metadata_number(r#"{"metadata": {"guardrails": 3}}"#)]
#[case::litellm_metadata_text(r#"{"litellm_metadata": {"guardrails": "a"}}"#)]
#[case::litellm_metadata_object(r#"{"metadata": {}, "litellm_metadata": {"guardrails": {}}}"#)]
fn requested_value_that_is_not_a_list_is_refused(#[case] request: &str) {
    assert_eq!(
        requested_guardrails(&data(request)),
        Err(GuardrailSelectionError::RequestedNotList)
    );
}

#[rstest]
#[case::text_entry(Some("pii"), r#"["pii"]"#, true)]
#[case::text_among_others(Some("pii"), r#"["a", "pii", "b"]"#, true)]
#[case::object_entry(Some("pii"), r#"[{"pii": {"extra_body": {}}}]"#, true)]
#[case::object_with_other_key(Some("pii"), r#"[{"other": {}, "pii": null}]"#, true)]
#[case::other_name(Some("pii"), r#"["pii2", {"other": {}}]"#, false)]
#[case::empty_list(Some("pii"), "[]", false)]
#[case::number_entry(Some("1"), "[1]", false)]
#[case::null_entry(Some("null"), "[null]", false)]
#[case::bool_entry(Some("true"), "[true]", false)]
#[case::nested_list_entry(Some("pii"), r#"[["pii"]]"#, false)]
#[case::no_name_text(None, r#"["pii", ""]"#, false)]
#[case::no_name_object(None, r#"[{"pii": {}}]"#, false)]
fn requested_entry_counts_for_a_text_or_a_key(
    #[case] name: Option<&str>,
    #[case] requested: &str,
    #[case] expected: bool,
) {
    let entries: Vec<GuardrailValue> = serde_json::from_str(requested).unwrap();
    assert_eq!(
        guardrail(name, None, false).is_requested_guardrail(&entries),
        expected
    );
}

#[rstest]
#[case::team_only(
    r#"{"metadata": {"user_api_key_team_metadata": {"a": 1}}}"#,
    r#"{"a": 1}"#
)]
#[case::key_only(r#"{"metadata": {"user_api_key_metadata": {"a": 2}}}"#, r#"{"a": 2}"#)]
#[case::key_over_team_key_by_key(
    r#"{"metadata": {"user_api_key_team_metadata": {"a": 1, "b": 1}, "user_api_key_metadata": {"a": 2, "c": 2}}}"#,
    r#"{"a": 2, "b": 1, "c": 2}"#
)]
#[case::litellm_metadata_replaces_metadata(
    r#"{"metadata": {"user_api_key_team_metadata": {"a": 1, "b": 1}}, "litellm_metadata": {"user_api_key_team_metadata": {"c": 3}}}"#,
    r#"{"c": 3}"#
)]
#[case::empty_object_leaves_earlier_team(
    r#"{"metadata": {"user_api_key_team_metadata": {"a": 1}}, "litellm_metadata": {"user_api_key_team_metadata": {}}}"#,
    r#"{"a": 1}"#
)]
#[case::empty_object_leaves_earlier_key(
    r#"{"metadata": {"user_api_key_metadata": {"a": 1}}, "litellm_metadata": {"user_api_key_metadata": {}}}"#,
    r#"{"a": 1}"#
)]
#[case::team_and_key_from_different_places(
    r#"{"metadata": {"user_api_key_team_metadata": {"a": 1, "b": 1}}, "litellm_metadata": {"user_api_key_metadata": {"a": 2}}}"#,
    r#"{"a": 2, "b": 1}"#
)]
#[case::non_object_metadata_skipped(
    r#"{"metadata": "user_api_key_metadata", "litellm_metadata": {"user_api_key_metadata": {"a": 1}}}"#,
    r#"{"a": 1}"#
)]
#[case::null_metadata_skipped(
    r#"{"metadata": null, "litellm_metadata": {"user_api_key_team_metadata": {"a": 1}}}"#,
    r#"{"a": 1}"#
)]
#[case::non_object_value_ignored(
    r#"{"metadata": {"user_api_key_team_metadata": "x", "user_api_key_metadata": [1]}}"#,
    "{}"
)]
#[case::root_keys_ignored(
    r#"{"user_api_key_metadata": {"a": 1}, "user_api_key_team_metadata": {"a": 1}}"#,
    "{}"
)]
#[case::nothing(r#"{"model": "m"}"#, "{}")]
fn admin_metadata_merges_in_place(#[case] request_json: &str, #[case] expected_json: &str) {
    let request = data(request_json);
    let expected = data(expected_json);
    let merged = admin_metadata(&request);
    assert_eq!(merged.len(), expected.len());
    for (name, wanted) in &expected {
        assert_eq!(merged.get(name.as_str()).copied(), Some(wanted));
    }
}

#[rstest]
#[case::team_true(
    r#"{"user_api_key_team_metadata": {"disable_global_guardrails": true}}"#,
    true
)]
#[case::key_true(
    r#"{"user_api_key_metadata": {"disable_global_guardrails": true}}"#,
    true
)]
#[case::key_false_beats_team(
    r#"{"user_api_key_team_metadata": {"disable_global_guardrails": true}, "user_api_key_metadata": {"disable_global_guardrails": false}}"#,
    false
)]
#[case::key_true_beats_team_false(
    r#"{"user_api_key_team_metadata": {"disable_global_guardrails": false}, "user_api_key_metadata": {"disable_global_guardrails": true}}"#,
    true
)]
#[case::text_true(
    r#"{"user_api_key_metadata": {"disable_global_guardrails": "true"}}"#,
    false
)]
#[case::number_one(
    r#"{"user_api_key_metadata": {"disable_global_guardrails": 1}}"#,
    false
)]
#[case::null(
    r#"{"user_api_key_metadata": {"disable_global_guardrails": null}}"#,
    false
)]
#[case::absent(r#"{"user_api_key_metadata": {"other": true}}"#, false)]
#[case::singular_name(
    r#"{"user_api_key_metadata": {"disable_global_guardrail": true}}"#,
    false
)]
fn admin_disable_flag_is_only_a_boolean_true(#[case] admin: &str, #[case] expected: bool) {
    let request = data(&format!(r#"{{"metadata": {admin}}}"#));
    assert_eq!(global_guardrails_disabled(&request), expected);
}

#[rstest]
#[case::root(r#"{"disable_global_guardrails": true}"#)]
#[case::root_singular(r#"{"disable_global_guardrail": true}"#)]
#[case::metadata_direct(r#"{"metadata": {"disable_global_guardrails": true}}"#)]
#[case::litellm_metadata_direct(r#"{"litellm_metadata": {"disable_global_guardrails": true}}"#)]
#[case::request_metadata_singular(
    r#"{"metadata": {"user_api_key_metadata": {"disable_global_guardrail": true}}}"#
)]
fn admin_disable_flag_outside_admin_metadata_counts_for_nothing(#[case] request: &str) {
    assert!(!global_guardrails_disabled(&data(request)));
}

#[rstest]
#[case::team_list(
    r#"{"user_api_key_team_metadata": {"opted_out_global_guardrails": ["a", "b"]}}"#,
    &["a", "b"]
)]
#[case::key_list_replaces_team(
    r#"{"user_api_key_team_metadata": {"opted_out_global_guardrails": ["a", "b"]}, "user_api_key_metadata": {"opted_out_global_guardrails": ["c"]}}"#,
    &["c"]
)]
#[case::key_without_the_list_keeps_team(
    r#"{"user_api_key_team_metadata": {"opted_out_global_guardrails": ["a"]}, "user_api_key_metadata": {"other": 1}}"#,
    &["a"]
)]
#[case::only_text_entries(
    r#"{"user_api_key_metadata": {"opted_out_global_guardrails": ["a", 1, null, ["b"], {"c": 1}, "d"]}}"#,
    &["a", "d"]
)]
#[case::text_value(r#"{"user_api_key_metadata": {"opted_out_global_guardrails": "a"}}"#, &[])]
#[case::object_value(r#"{"user_api_key_metadata": {"opted_out_global_guardrails": {"a": 1}}}"#, &[])]
#[case::null_value(r#"{"user_api_key_metadata": {"opted_out_global_guardrails": null}}"#, &[])]
#[case::absent(r#"{"user_api_key_metadata": {"other": 1}}"#, &[])]
fn admin_opted_out_list_reads_the_text_entries(#[case] admin: &str, #[case] expected: &[&str]) {
    let request = data(&format!(r#"{{"litellm_metadata": {admin}}}"#));
    assert_eq!(opted_out_global_guardrails(&request), expected);
}

#[rstest]
#[case::root(r#"{"opted_out_global_guardrails": ["g"]}"#)]
#[case::metadata_direct(r#"{"metadata": {"opted_out_global_guardrails": ["g"]}}"#)]
#[case::litellm_metadata_direct(r#"{"litellm_metadata": {"opted_out_global_guardrails": ["g"]}}"#)]
#[case::request_guardrails_list(r#"{"guardrails": ["g"], "metadata": {"guardrails": ["g"]}}"#)]
fn admin_opt_out_outside_admin_metadata_counts_for_nothing(#[case] request: &str) {
    assert!(opted_out_global_guardrails(&data(request)).is_empty());
}

#[rstest]
#[case::matching_hook(Some(r#""pre_call""#), PRE, "{}", true)]
#[case::no_hook(None, POST, "{}", true)]
#[case::other_hook(Some(r#""pre_call""#), POST, "{}", false)]
#[case::list_hook(Some(r#"["pre_call", "post_call"]"#), POST, "{}", true)]
#[case::empty_request_list_cannot_stop_it(
    Some(r#""pre_call""#),
    PRE,
    r#"{"guardrails": []}"#,
    true
)]
#[case::other_request_list_cannot_stop_it(
    Some(r#""pre_call""#),
    PRE,
    r#"{"guardrails": ["other"]}"#,
    true
)]
#[case::bad_request_list_is_never_read(Some(r#""pre_call""#), PRE, r#"{"guardrails": 5}"#, true)]
#[case::logging_only_needs_a_matching_hook(
    Some(r#""pre_call""#),
    LOGGING,
    r#"{"guardrails": ["g"]}"#,
    false
)]
fn gate_default_on_guardrail_follows_the_hook(
    #[case] hook: Option<&str>,
    #[case] event: GuardrailEventHooks,
    #[case] request: &str,
    #[case] expected: bool,
) {
    let base = guardrail(Some("g"), hook, true);
    assert_eq!(runs(&base, &data(request), event), Ok(expected));
}

#[rstest]
#[case::team_list(r#"{"user_api_key_team_metadata": {"opted_out_global_guardrails": ["g"]}}"#)]
#[case::key_list(r#"{"user_api_key_metadata": {"opted_out_global_guardrails": ["x", "g"]}}"#)]
#[case::team_disable(r#"{"user_api_key_team_metadata": {"disable_global_guardrails": true}}"#)]
#[case::key_disable(r#"{"user_api_key_metadata": {"disable_global_guardrails": true}}"#)]
fn gate_default_on_guardrail_stops_on_an_admin_opt_out(#[case] admin: &str) {
    let base = guardrail(Some("g"), Some(r#""pre_call""#), true);
    for location in ["metadata", "litellm_metadata"] {
        let request = data(&format!(r#"{{"{location}": {admin}}}"#));
        assert_eq!(runs(&base, &request, PRE), Ok(false));
    }
}

#[rstest]
#[case::other_name_listed(
    r#"{"user_api_key_metadata": {"opted_out_global_guardrails": ["other"]}}"#
)]
#[case::flag_false(r#"{"user_api_key_metadata": {"disable_global_guardrails": false}}"#)]
#[case::key_false_beats_team_true(
    r#"{"user_api_key_team_metadata": {"disable_global_guardrails": true}, "user_api_key_metadata": {"disable_global_guardrails": false}}"#
)]
#[case::key_list_replaces_team_list(
    r#"{"user_api_key_team_metadata": {"opted_out_global_guardrails": ["g"]}, "user_api_key_metadata": {"opted_out_global_guardrails": ["other"]}}"#
)]
#[case::text_true_is_not_true(
    r#"{"user_api_key_metadata": {"disable_global_guardrails": "true"}}"#
)]
fn gate_default_on_guardrail_keeps_running_when_nothing_opts_it_out(#[case] admin: &str) {
    let base = guardrail(Some("g"), Some(r#""pre_call""#), true);
    let request = data(&format!(r#"{{"metadata": {admin}}}"#));
    assert_eq!(runs(&base, &request, PRE), Ok(true));
}

#[rstest]
#[case::caller_flag_at_root(r#"{"disable_global_guardrails": true}"#)]
#[case::caller_singular_flag(r#"{"metadata": {"disable_global_guardrail": true}}"#)]
#[case::caller_list_in_metadata(r#"{"metadata": {"opted_out_global_guardrails": ["g"]}}"#)]
#[case::caller_list_at_root(r#"{"opted_out_global_guardrails": ["g"]}"#)]
fn gate_default_on_guardrail_ignores_opt_outs_a_caller_sent(#[case] request: &str) {
    let base = guardrail(Some("g"), Some(r#""pre_call""#), true);
    assert_eq!(runs(&base, &data(request), PRE), Ok(true));
}

#[test]
fn gate_default_on_guardrail_without_a_name_is_never_opted_out_by_name() {
    let base = guardrail(None, None, true);
    let request = data(
        r#"{"metadata": {"user_api_key_metadata": {"opted_out_global_guardrails": ["", "None"]}}}"#,
    );
    assert_eq!(runs(&base, &request, PRE), Ok(true));
}

#[rstest]
#[case::not_requested(Some(r#""pre_call""#), PRE, "{}", false)]
#[case::empty_list(Some(r#""pre_call""#), PRE, r#"{"guardrails": []}"#, false)]
#[case::other_name(Some(r#""pre_call""#), PRE, r#"{"guardrails": ["other"]}"#, false)]
#[case::requested_by_text(Some(r#""pre_call""#), PRE, r#"{"guardrails": ["g"]}"#, true)]
#[case::requested_by_object(Some(r#""pre_call""#), PRE, r#"{"guardrails": [{"g": {}}]}"#, true)]
#[case::requested_in_metadata(
    Some(r#""pre_call""#),
    PRE,
    r#"{"metadata": {"guardrails": ["g"]}}"#,
    true
)]
#[case::requested_in_litellm_metadata(
    Some(r#""pre_call""#),
    PRE,
    r#"{"litellm_metadata": {"guardrails": ["g"]}}"#,
    true
)]
#[case::root_list_hides_metadata_request(
    Some(r#""pre_call""#),
    PRE,
    r#"{"guardrails": [], "metadata": {"guardrails": ["g"]}}"#,
    false
)]
#[case::requested_for_another_hook(Some(r#""pre_call""#), POST, r#"{"guardrails": ["g"]}"#, false)]
#[case::requested_list_hook(
    Some(r#"["pre_call", "post_call"]"#),
    POST,
    r#"{"guardrails": ["g"]}"#,
    true
)]
#[case::logging_only_skips_the_request_check(Some(r#""logging_only""#), LOGGING, "{}", true)]
#[case::logging_only_in_a_list(Some(r#"["pre_call", "logging_only"]"#), LOGGING, "{}", true)]
#[case::logging_only_still_needs_the_hook(Some(r#""pre_call""#), LOGGING, "{}", false)]
#[case::no_hook_runs_everywhere(None, POST, "{}", true)]
#[case::no_hook_runs_on_logging(None, LOGGING, "{}", true)]
#[case::no_hook_ignores_an_empty_request_list(None, PRE, r#"{"guardrails": []}"#, true)]
#[case::no_hook_never_reads_a_bad_request_list(None, PRE, r#"{"guardrails": 5}"#, true)]
#[case::logging_only_never_reads_a_bad_request_list(
    Some(r#""logging_only""#),
    LOGGING,
    r#"{"guardrails": 5}"#,
    true
)]
fn gate_requested_guardrail_needs_its_request_and_its_hook(
    #[case] hook: Option<&str>,
    #[case] event: GuardrailEventHooks,
    #[case] request: &str,
    #[case] expected: bool,
) {
    let base = guardrail(Some("g"), hook, false);
    assert_eq!(runs(&base, &data(request), event), Ok(expected));
}

#[rstest]
#[case::no_hook(None)]
#[case::hook(Some(r#""pre_call""#))]
fn gate_opt_outs_never_stop_a_guardrail_that_is_not_default_on(#[case] hook: Option<&str>) {
    let base = guardrail(Some("g"), hook, false);
    let request = data(
        r#"{"guardrails": ["g"], "metadata": {"user_api_key_metadata": {"opted_out_global_guardrails": ["g"], "disable_global_guardrails": true}}}"#,
    );
    assert_eq!(runs(&base, &request, PRE), Ok(true));
}

#[test]
fn gate_requested_guardrail_with_a_hook_refuses_a_bad_request_list() {
    let base = guardrail(Some("g"), Some(r#""pre_call""#), false);
    assert_eq!(
        runs(&base, &data(r#"{"guardrails": "g"}"#), PRE),
        Err(GuardrailSelectionError::RequestedNotList)
    );
}

#[test]
fn gate_guardrail_without_a_name_is_never_requested() {
    let base = guardrail(None, Some(r#""pre_call""#), false);
    let request = data(r#"{"guardrails": ["", {"": {}}]}"#);
    assert_eq!(runs(&base, &request, PRE), Ok(false));
}

#[rstest]
#[case::default_on(true, Some(r#""pre_call""#), "{}")]
#[case::requested(false, Some(r#""pre_call""#), r#"{"guardrails": ["g"]}"#)]
#[case::no_hook(false, None, "{}")]
#[case::default_on_without_hook(true, None, "{}")]
fn suppressed_name_gives_false_ahead_of_every_other_step(
    #[case] default_on: bool,
    #[case] hook: Option<&str>,
    #[case] request_json: &str,
) {
    let base = guardrail(Some("g"), hook, default_on);
    let request = data(request_json);
    assert_eq!(runs(&base, &request, PRE), Ok(true));
    let silenced = base.should_run_guardrail(&request, PRE, &names(&["other", "g"]));
    assert_eq!(silenced, Ok(false));
    let elsewhere = base.should_run_guardrail(&request, PRE, &names(&["other"]));
    assert_eq!(elsewhere, Ok(true));
}

#[test]
fn suppressed_name_comes_before_the_tag_error_and_the_request_check() {
    let tagged = guardrail(Some("g"), Some(r#"{"tags": {"a": "pre_call"}}"#), true);
    let silenced = names(&["g"]);
    assert_eq!(
        tagged.should_run_guardrail(&Data::new(), PRE, &silenced),
        Ok(false)
    );
    let requested = guardrail(Some("g"), Some(r#""pre_call""#), false);
    let bad = data(r#"{"guardrails": 5}"#);
    assert_eq!(
        requested.should_run_guardrail(&bad, PRE, &silenced),
        Ok(false)
    );
}

#[test]
fn suppressed_guardrail_without_a_name_is_never_suppressed() {
    let base = guardrail(None, None, true);
    let request = Data::new();
    assert_eq!(
        base.should_run_guardrail(&request, PRE, &names(&["", "None"])),
        Ok(true)
    );
}

#[rstest]
#[case::metadata_list(
    r#"{"metadata": {"suppressed_guardrails": ["g"], "suppressed_compression_guardrails": ["g"]}}"#
)]
#[case::litellm_metadata_list(
    r#"{"litellm_metadata": {"suppressed_compression_guardrails": ["g"]}}"#
)]
#[case::root_list(
    r#"{"suppressed_guardrails": ["g"], "suppressed_compression_guardrails": ["g"]}"#
)]
#[case::admin_metadata(
    r#"{"metadata": {"user_api_key_metadata": {"suppressed_compression_guardrails": ["g"]}}}"#
)]
fn suppressed_names_placed_in_data_suppress_nothing(#[case] request: &str) {
    let base = guardrail(Some("g"), None, true);
    assert_eq!(runs(&base, &data(request), PRE), Ok(true));
}

#[rstest]
#[case::default_on(true, "{}")]
#[case::requested(false, r#"{"guardrails": ["g"]}"#)]
fn tag_matching_hook_is_an_error_and_never_false(#[case] default_on: bool, #[case] request: &str) {
    for hook in [
        r#"{"tags": {"a": "pre_call"}}"#,
        r#"{"tags": {"a": ["post_call", "pre_call"]}}"#,
        r#"{"tags": {"a": "post_call"}, "default": "pre_call"}"#,
        r#"{"tags": {"a": "post_call"}, "default": ["pre_call"]}"#,
    ] {
        let base = guardrail(Some("g"), Some(hook), default_on);
        assert_eq!(
            runs(&base, &data(request), PRE),
            Err(GuardrailSelectionError::TagMode {
                guardrail_name: Some("g".to_owned())
            })
        );
    }
}

#[test]
fn tag_error_names_no_guardrail_when_it_has_no_name() {
    let base = guardrail(None, Some(r#"{"tags": {"a": "pre_call"}}"#), true);
    assert_eq!(
        runs(&base, &Data::new(), PRE),
        Err(GuardrailSelectionError::TagMode {
            guardrail_name: None
        })
    );
}

#[rstest]
#[case::default_on(true, "{}")]
#[case::requested(false, r#"{"guardrails": ["g"]}"#)]
fn tag_hook_for_another_event_gives_false(#[case] default_on: bool, #[case] request: &str) {
    let base = guardrail(
        Some("g"),
        Some(r#"{"tags": {"a": "pre_call"}, "default": "post_call"}"#),
        default_on,
    );
    assert_eq!(
        runs(&base, &data(request), GuardrailEventHooks::DuringCall),
        Ok(false)
    );
}

#[rstest]
#[case::opted_out(
    r#"{"metadata": {"user_api_key_metadata": {"opted_out_global_guardrails": ["g"]}}}"#
)]
#[case::disabled(
    r#"{"metadata": {"user_api_key_team_metadata": {"disable_global_guardrails": true}}}"#
)]
fn tag_default_on_opt_out_gives_false_before_the_error(#[case] request: &str) {
    let base = guardrail(Some("g"), Some(r#"{"tags": {"a": "pre_call"}}"#), true);
    assert_eq!(runs(&base, &data(request), PRE), Ok(false));
}

#[rstest]
#[case::not_requested("{}")]
#[case::other_requested(r#"{"guardrails": ["other"]}"#)]
fn tag_hook_that_was_not_requested_gives_false_before_the_error(#[case] request: &str) {
    let base = guardrail(Some("g"), Some(r#"{"tags": {"a": "pre_call"}}"#), false);
    assert_eq!(runs(&base, &data(request), PRE), Ok(false));
}

#[test]
fn tag_hook_on_logging_only_for_a_matching_tag_is_an_error_without_a_request() {
    let base = guardrail(Some("g"), Some(r#"{"tags": {"a": "logging_only"}}"#), false);
    assert_eq!(
        runs(&base, &Data::new(), LOGGING),
        Err(GuardrailSelectionError::TagMode {
            guardrail_name: Some("g".to_owned())
        })
    );
}

#[rstest]
#[case::text(r#""pre_call""#)]
#[case::list(r#"["pre_call"]"#)]
fn tag_text_and_list_hooks_never_give_the_error(#[case] hook: &str) {
    let base = guardrail(Some("g"), Some(hook), true);
    assert_eq!(runs(&base, &Data::new(), PRE), Ok(true));
}

#[rstest]
#[case::first_object_entry(
    r#"{"guardrails": [{"g": {"extra_body": {"a": 1}}}, {"g": {"extra_body": {"a": 2}}}]}"#,
    r#"{"a": 1}"#
)]
#[case::text_entry_skipped(
    r#"{"guardrails": ["g", {"g": {"extra_body": {"a": 1}}}]}"#,
    r#"{"a": 1}"#
)]
#[case::other_entries_skipped(
    r#"{"guardrails": [{"other": {"extra_body": {"a": 9}}}, 5, null, {"g": {"extra_body": {"a": 1}}}]}"#,
    r#"{"a": 1}"#
)]
#[case::from_metadata(
    r#"{"metadata": {"guardrails": [{"g": {"extra_body": {"a": 1}}}]}}"#,
    r#"{"a": 1}"#
)]
#[case::from_litellm_metadata(
    r#"{"litellm_metadata": {"guardrails": [{"g": {"extra_body": {"a": 1}}}]}}"#,
    r#"{"a": 1}"#
)]
#[case::first_object_decides_even_without_extra_body(
    r#"{"guardrails": [{"g": {}}, {"g": {"extra_body": {"a": 2}}}]}"#,
    "{}"
)]
#[case::first_object_decides_even_when_null(
    r#"{"guardrails": [{"g": null}, {"g": {"extra_body": {"a": 2}}}]}"#,
    "{}"
)]
#[case::empty_extra_body(r#"{"guardrails": [{"g": {"extra_body": {}}}]}"#, "{}")]
#[case::missing_extra_body(r#"{"guardrails": [{"g": {"other": 1}}]}"#, "{}")]
#[case::null_extra_body(r#"{"guardrails": [{"g": {"extra_body": null}}]}"#, "{}")]
#[case::null_entry_value(r#"{"guardrails": [{"g": null}]}"#, "{}")]
#[case::text_entry_only(r#"{"guardrails": ["g"]}"#, "{}")]
#[case::other_guardrail_only(r#"{"guardrails": [{"other": {"extra_body": {"a": 1}}}]}"#, "{}")]
#[case::no_request("{}", "{}")]
fn dynamic_params_come_from_the_first_object_entry_naming_the_guardrail(
    #[case] request: &str,
    #[case] expected: &str,
) {
    let base = guardrail(Some("g"), None, false);
    assert_eq!(
        base.dynamic_request_body_params(&data(request)),
        Ok(data(expected))
    );
}

#[test]
fn dynamic_params_keep_nested_values_and_kinds() {
    let base = guardrail(Some("g"), None, false);
    let request = data(
        r#"{"guardrails": [{"g": {"extra_body": {"t": 0.5, "n": [1, "x"], "o": {"k": null}}}}]}"#,
    );
    let answer = base.dynamic_request_body_params(&request).unwrap();
    assert_eq!(answer.get("t"), Some(&value("0.5")));
    assert_eq!(answer.get("n"), Some(&value(r#"[1, "x"]"#)));
    assert_eq!(answer.get("o"), Some(&value(r#"{"k": null}"#)));
}

#[test]
fn dynamic_params_of_a_guardrail_without_a_name_are_empty() {
    let base = guardrail(None, None, false);
    let request = data(r#"{"guardrails": [{"": {"extra_body": {"a": 1}}}, "pii"]}"#);
    assert_eq!(base.dynamic_request_body_params(&request), Ok(Data::new()));
}

#[rstest]
#[case::text_value(r#"{"guardrails": [{"g": "x"}]}"#)]
#[case::number_value(r#"{"guardrails": [{"g": 1}]}"#)]
#[case::bool_value(r#"{"guardrails": [{"g": true}]}"#)]
#[case::list_value(r#"{"guardrails": [{"g": []}]}"#)]
#[case::text_extra_body(r#"{"guardrails": [{"g": {"extra_body": "x"}}]}"#)]
#[case::list_extra_body(r#"{"guardrails": [{"g": {"extra_body": [1]}}]}"#)]
#[case::number_extra_body(r#"{"guardrails": [{"g": {"extra_body": 0}}]}"#)]
#[case::bool_extra_body(r#"{"guardrails": [{"g": {"extra_body": false}}]}"#)]
#[case::bad_value_after_a_text_entry(r#"{"guardrails": ["g", {"g": 1}]}"#)]
fn dynamic_params_that_are_not_an_object_are_refused(#[case] request: &str) {
    let base = guardrail(Some("g"), None, false);
    assert_eq!(
        base.dynamic_request_body_params(&data(request)),
        Err(GuardrailSelectionError::DynamicParamsNotObject {
            guardrail_name: "g".to_owned()
        })
    );
}

#[test]
fn dynamic_params_leave_alone_a_bad_value_for_another_guardrail() {
    let base = guardrail(Some("g"), None, false);
    let request = data(r#"{"guardrails": [{"other": 1}, {"g": {"extra_body": {"a": 1}}}]}"#);
    assert_eq!(
        base.dynamic_request_body_params(&request),
        Ok(data(r#"{"a": 1}"#))
    );
}

#[rstest]
#[case::named(Some("g"))]
#[case::nameless(None)]
fn dynamic_params_refuse_a_request_list_that_is_not_a_list(#[case] name: Option<&str>) {
    let base = guardrail(name, None, false);
    assert_eq!(
        base.dynamic_request_body_params(&data(r#"{"guardrails": "g"}"#)),
        Err(GuardrailSelectionError::RequestedNotList)
    );
}

#[rstest]
#[case::tag_mode(
    GuardrailSelectionError::TagMode { guardrail_name: Some("g".to_owned()) },
    500,
    "Guardrail g uses a tag-based event hook, which betterlite does not support"
)]
#[case::tag_mode_without_name(
    GuardrailSelectionError::TagMode { guardrail_name: None },
    500,
    "Guardrail None uses a tag-based event hook, which betterlite does not support"
)]
#[case::requested_not_list(
    GuardrailSelectionError::RequestedNotList,
    400,
    "The requested guardrails must be a list"
)]
#[case::dynamic_not_object(
    GuardrailSelectionError::DynamicParamsNotObject { guardrail_name: "g".to_owned() },
    400,
    "The request options for guardrail g must be an object"
)]
fn failure_carries_the_status_and_the_error_text(
    #[case] error: GuardrailSelectionError,
    #[case] status: u16,
    #[case] detail: &str,
) {
    assert_eq!(
        GuardrailFailure::from(error),
        GuardrailFailure::Http {
            status,
            detail: detail.to_owned()
        }
    );
}
