#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks as Event;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use rust_decimal::Decimal;

use super::error::{GuardrailRecordError, GuardrailSpanError};
use super::invocation::{GuardrailHookName as Hook, GuardrailInvocation};
use super::messages::messages_for_call_type;
use super::metadata::add_guardrail_record;
use super::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
    GuardrailStatus, GuardrailTracingDetail,
};
use super::sanitize::{MAX_NULL_DROP_DEPTH, drop_nulls, sanitize_guardrail_response};
use super::summary::summarize_guardrail_response;
use super::sync::sync_log_target;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_base::settings::GuardrailSettings;

type Body = BTreeMap<String, GuardrailValue>;

const RECORDS: &str = "standard_logging_guardrail_information";
const ENTRY: &str = r#"{"guardrail_name":"pii","guardrail_status":"success"}"#;
const SYNCED: &str = r#"{"metadata":{"@R":[@E]}}"#;
const HOOK_LIST: &str = r#"["pre_call","post_call"]"#;
const TAGGED_HOOK: &str = r#"{"tags":{"health":"pre_call"},"default":["post_call"]}"#;

// "@R" stands for the records key, "@E" for ENTRY and "@M" for a masked bearer token
fn text(source: &str) -> String {
    source
        .replace("@R", RECORDS)
        .replace("@E", ENTRY)
        .replace("@M", &format!("Bear{}ijkl", "*".repeat(14)))
}

// serde_json hands a parsed float to the untagged enum as this one-key object
fn plain(value: GuardrailValue) -> GuardrailValue {
    use GuardrailValue::{
        Array, Boolean, Decimal, Null, Object, SignedInteger, Text, UnsignedInteger,
    };
    match value {
        Object(fields) => {
            if let Some(Text(digits)) = fields.get("$serde_json::private::Number") {
                return Decimal(digits.parse().unwrap());
            }
            Object(
                fields
                    .into_iter()
                    .map(|(key, field)| (key, plain(field)))
                    .collect(),
            )
        }
        Array(items) => Array(items.into_iter().map(plain).collect()),
        scalar @ (Null | Boolean(_) | SignedInteger(_) | UnsignedInteger(_) | Decimal(_)
        | Text(_)) => scalar,
    }
}

fn gv(source: &str) -> GuardrailValue {
    plain(serde_json::from_str(&text(source)).unwrap())
}

fn body(source: &str) -> Body {
    let GuardrailValue::Object(fields) = gv(source) else {
        panic!("not an object: {source}");
    };
    fields
}

fn field(value: &GuardrailValue, key: &str) -> GuardrailValue {
    let GuardrailValue::Object(fields) = value else {
        panic!("not an object: {value:?}");
    };
    fields[key].clone()
}

fn list(items: Vec<GuardrailValue>) -> GuardrailValue {
    GuardrailValue::Array(items)
}

fn at(millis: u64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_millis(millis))
        .unwrap()
}

fn nested(levels: usize, leaf: GuardrailValue) -> GuardrailValue {
    let mut value = leaf;
    for _ in 0..levels {
        value = GuardrailValue::Object(BTreeMap::from([("a".to_owned(), value)]));
    }
    value
}

fn expected_entry(overrides: &str) -> GuardrailValue {
    let mut entry = body(
        r#"{"guardrail_name":null,"guardrail_provider":null,"guardrail_mode":null,
        "guardrail_response":{},"guardrail_status":"success","start_time":null,
        "end_time":null,"duration":null,"masked_entity_count":null}"#,
    );
    entry.extend(body(overrides));
    GuardrailValue::Object(entry)
}

// a metadata bucket holding the extra keys and the record list
fn bucket(extra: &str, entries: Vec<GuardrailValue>) -> GuardrailValue {
    let mut fields = body(extra);
    fields.insert(RECORDS.to_owned(), list(entries));
    GuardrailValue::Object(fields)
}

fn only(key: &str, value: GuardrailValue) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([(key.to_owned(), value)]))
}

fn stored(data: &Body, bucket_key: &str) -> Vec<GuardrailValue> {
    let Some(GuardrailValue::Object(fields)) = data.get(bucket_key) else {
        panic!("no {bucket_key} object in {data:?}");
    };
    match fields.get(RECORDS) {
        Some(GuardrailValue::Array(entries)) => entries.iter().cloned().map(plain).collect(),
        other => panic!("expected a record list, got {other:?}"),
    }
}

fn stored_count(data: &Body) -> usize {
    if data.contains_key("metadata") {
        stored(data, "metadata").len()
    } else {
        0
    }
}

fn success(response: &str) -> GuardrailRecordInput {
    GuardrailRecordInput {
        guardrail_response: gv(response),
        guardrail_status: GuardrailStatus::Success,
        start_time: None,
        end_time: None,
        duration: None,
        masked_entity_count: None,
        guardrail_provider: None,
        event_type: None,
        tracing_detail: None,
    }
}

struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

struct FailingSink;

impl GuardrailSpanSink for FailingSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Err(GuardrailSpanError::Unavailable)
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<GuardrailInvocationRecord>>);

impl GuardrailSpanSink for RecordingSink {
    fn emit(&self, record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        self.0
            .lock()
            .map_err(|_poisoned| GuardrailSpanError::Unavailable)?
            .push(record.clone());
        Ok(())
    }
}

struct Run {
    base: GuardrailBase,
    data: Body,
    target: GuardrailLogTarget,
}

impl Run {
    fn new(base: GuardrailBase, data: &str) -> Self {
        Self {
            base,
            data: body(data),
            target: GuardrailLogTarget {
                litellm_params: Some(Body::new()),
                model_call_params: Some(Body::new()),
            },
        }
    }

    fn context<'run>(
        &'run mut self,
        sink: &'run dyn GuardrailSpanSink,
    ) -> GuardrailRecordContext<'run> {
        GuardrailRecordContext {
            base: &self.base,
            request_data: &mut self.data,
            log_target: Some(&mut self.target),
            sink,
        }
    }

    fn add(&mut self, input: GuardrailRecordInput) {
        add_guardrail_record(&mut self.context(&DiscardSink), input).unwrap();
    }

    fn begin(&self, hook: Hook, millis: u64) -> GuardrailInvocation {
        GuardrailInvocation::begin(hook, &self.data, None, at(millis)).unwrap()
    }

    fn succeed(&mut self, invocation: GuardrailInvocation, response: Option<&str>, millis: u64) {
        let value = response.map(gv);
        let context = &mut self.context(&DiscardSink);
        invocation
            .finish_success(context, value.as_ref(), at(millis))
            .unwrap();
    }

    fn fail(
        &mut self,
        invocation: GuardrailInvocation,
        failure: &GuardrailFailure,
        millis: u64,
    ) -> GuardrailFailure {
        let context = &mut self.context(&DiscardSink);
        invocation
            .finish_failure(context, failure.clone(), at(millis))
            .unwrap()
    }
}

fn plain_run(data: &str) -> Run {
    Run::new(
        GuardrailBase {
            settings: GuardrailSettings::default(),
            mode: None,
        },
        data,
    )
}

fn named_run(data: &str) -> Run {
    let mut run = plain_run(data);
    run.base.settings.guardrail_name = Some("pii".to_owned());
    run.base.settings.event_hook = Some(serde_json::from_str(r#""post_call""#).unwrap());
    run
}

fn own_info_run(records_own_information: bool, data: &str) -> Run {
    let mut run = named_run(data);
    run.base.settings.switches.records_own_information = records_own_information;
    run
}

#[test]
fn record_writes_nine_base_keys_null_when_unset() {
    let mut run = plain_run("{}");
    run.add(success("{}"));
    assert_eq!(
        plain(GuardrailValue::Object(run.data)),
        only("metadata", bucket("{}", vec![expected_entry("{}")]))
    );
}

#[test]
fn record_keeps_the_values_it_is_given() {
    let mut run = named_run("{}");
    run.add(GuardrailRecordInput {
        guardrail_status: GuardrailStatus::GuardrailFlagged,
        start_time: Some(1.5_f64),
        end_time: Some(2.5_f64),
        duration: Some(1.0_f64),
        masked_entity_count: Some(BTreeMap::from([("EMAIL".to_owned(), 2)])),
        guardrail_provider: Some("presidio".to_owned()),
        ..success(r#"{"verdict":"flagged"}"#)
    });
    let overrides = r#"{"guardrail_name":"pii","guardrail_provider":"presidio",
        "guardrail_mode":"post_call","guardrail_response":{"verdict":"flagged"},
        "guardrail_status":"guardrail_flagged","start_time":1.5,"end_time":2.5,
        "duration":1.0,"masked_entity_count":{"EMAIL":2}}"#;
    assert_eq!(
        plain(GuardrailValue::Object(run.data)),
        only("metadata", bucket("{}", vec![expected_entry(overrides)]))
    );
}

#[test]
fn record_status_uses_wire_names() {
    let statuses = [
        GuardrailStatus::NotRun,
        GuardrailStatus::GuardrailFailedToRespond,
        GuardrailStatus::GuardrailIntervened,
        GuardrailStatus::GuardrailFlagged,
        GuardrailStatus::Success,
    ];
    assert_eq!(
        serde_json::to_string(&statuses).unwrap(),
        r#"["not_run","guardrail_failed_to_respond","guardrail_intervened","guardrail_flagged","success"]"#
    );
}

#[test]
fn record_writes_every_set_tracing_field_flat() {
    let mut run = plain_run("{}");
    run.add(GuardrailRecordInput {
        tracing_detail: Some(GuardrailTracingDetail {
            guardrail_id: Some("g-1".to_owned()),
            policy_template: Some("template".to_owned()),
            detection_method: Some("regex".to_owned()),
            confidence_score: Some(0.5_f64),
            classification: Some(gv(r#""pii""#)),
            match_details: Some(gv(r#"[{"type":"EMAIL"}]"#)),
            patterns_checked: Some(3),
            alert_recipients: Some(vec!["ops@example.com".to_owned()]),
            risk_score: Some(0.25_f64),
            violation_categories: Some(vec!["privacy".to_owned()]),
            guardrail_action: Some("BLOCKED".to_owned()),
            guardrail_usage: Some(BTreeMap::from([("tokens".to_owned(), 7)])),
            guardrail_cost: Some(Decimal::new(75, 2)),
            guardrail_cost_by_unit: Some(BTreeMap::from([
                ("text".to_owned(), Some(Decimal::new(5, 1))),
                ("image".to_owned(), None),
            ])),
            guardrail_cost_in_spend: Some(true),
        }),
        ..success("{}")
    });
    let overrides = r#"{"guardrail_id":"g-1","policy_template":"template",
        "detection_method":"regex","confidence_score":0.5,"classification":"pii",
        "match_details":[{"type":"EMAIL"}],"patterns_checked":3,
        "alert_recipients":["ops@example.com"],"risk_score":0.25,
        "violation_categories":["privacy"],"guardrail_action":"BLOCKED",
        "guardrail_usage":{"tokens":7},"guardrail_cost":0.75,
        "guardrail_cost_by_unit":{"text":0.5,"image":null},"guardrail_cost_in_spend":true}"#;
    assert_eq!(
        stored(&run.data, "metadata"),
        vec![expected_entry(overrides)]
    );
}

#[rstest]
#[case::event_type_wins(Some(Event::PreCall), Some(r#""post_call""#), r#""pre_call""#)]
#[case::during_call(Some(Event::DuringCall), None, r#""during_call""#)]
#[case::single_hook(None, Some(r#""post_call""#), r#""post_call""#)]
#[case::hook_list(None, Some(HOOK_LIST), HOOK_LIST)]
#[case::tagged(None, Some(TAGGED_HOOK), TAGGED_HOOK)]
#[case::unset(None, None, "null")]
fn record_mode_follows_event_type_then_configured_hook(
    #[case] event_type: Option<Event>,
    #[case] configured: Option<&str>,
    #[case] expected: &str,
) {
    let mut run = plain_run("{}");
    run.base.settings.event_hook = configured.map(|hook| serde_json::from_str(hook).unwrap());
    run.add(GuardrailRecordInput {
        event_type,
        ..success("{}")
    });
    assert_eq!(
        field(&stored(&run.data, "metadata")[0], "guardrail_mode"),
        gv(expected)
    );
}

#[test]
fn record_hands_the_sink_the_record_and_ignores_a_failing_sink() {
    let mut failing = plain_run("{}");
    add_guardrail_record(&mut failing.context(&FailingSink), success("{}")).unwrap();
    assert_eq!(stored(&failing.data, "metadata").len(), 1);
    let mut run = named_run("{}");
    let sink = RecordingSink::default();
    let input = GuardrailRecordInput {
        guardrail_provider: Some("lakera".to_owned()),
        ..success(r#"{"a":1}"#)
    };
    add_guardrail_record(&mut run.context(&sink), input).unwrap();
    let records = sink.0.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].guardrail_name.as_deref(), Some("pii"));
    assert_eq!(records[0].guardrail_provider.as_deref(), Some("lakera"));
    assert_eq!(records[0].guardrail_response, gv(r#"{"a":1}"#));
}

#[test]
fn record_sanitizes_the_response_it_stores() {
    let mut run = plain_run("{}");
    run.add(success(r#"{"match":"secret","gone":null,"kept":1}"#));
    assert_eq!(
        field(&stored(&run.data, "metadata")[0], "guardrail_response"),
        gv(r#"{"match":"[REDACTED]","kept":1}"#)
    );
}

#[test]
fn record_syncs_both_log_targets() {
    let mut run = plain_run("{}");
    run.add(success("{}"));
    assert_eq!(run.target.litellm_params, Some(run.data.clone()));
    assert_eq!(run.target.model_call_params, Some(run.data));
}

#[rstest]
#[case::litellm_metadata_wins(
    r#"{"litellm_metadata":{"a":1},"metadata":{"b":2}}"#,
    "litellm_metadata",
    r#"{"a":1}"#
)]
#[case::metadata(r#"{"metadata":{"b":2}}"#, "metadata", r#"{"b":2}"#)]
#[case::no_bucket(r#"{"model":"m"}"#, "metadata", "{}")]
#[case::text_bucket_replaced(r#"{"metadata":"text"}"#, "metadata", "{}")]
#[case::null_litellm_metadata_replaced(
    r#"{"litellm_metadata":null,"metadata":{"b":2}}"#,
    "litellm_metadata",
    "{}"
)]
fn metadata_picks_the_bucket(#[case] before: &str, #[case] key: &str, #[case] kept: &str) {
    let mut run = plain_run(before);
    run.add(success("{}"));
    let mut expected = body(before);
    expected.insert(key.to_owned(), bucket(kept, vec![expected_entry("{}")]));
    assert_eq!(run.data, expected);
}

#[rstest]
#[case::missing(None, "[]")]
#[case::null(Some("null"), "[]")]
#[case::empty_list(Some("[]"), "[]")]
#[case::list(Some(r#"[{"n":1}]"#), r#"[{"n":1}]"#)]
#[case::object(Some(r#"{"n":1}"#), r#"[{"n":1}]"#)]
#[case::text(Some(r#""old""#), r#"["old"]"#)]
fn metadata_appends_to_what_is_there(#[case] existing: Option<&str>, #[case] before: &str) {
    let mut run = plain_run(r#"{"metadata":{"keep":true}}"#);
    if let (Some(GuardrailValue::Object(fields)), Some(value)) =
        (run.data.get_mut("metadata"), existing)
    {
        fields.insert(RECORDS.to_owned(), gv(value));
    }
    run.add(success("{}"));
    let GuardrailValue::Array(mut entries) = gv(before) else {
        panic!("the expected list is not a list");
    };
    entries.push(expected_entry("{}"));
    assert_eq!(run.data["metadata"], bucket(r#"{"keep":true}"#, entries));
}

fn targets(litellm_params: Option<&str>, model_call_params: Option<&str>) -> GuardrailLogTarget {
    GuardrailLogTarget {
        litellm_params: litellm_params.map(body),
        model_call_params: model_call_params.map(body),
    }
}

#[rstest]
#[case::missing("{}", r#"{"metadata":{"@R":[@E]}}"#)]
#[case::null(r#"{"metadata":null}"#, r#"{"metadata":{"@R":[@E]}}"#)]
#[case::existing(r#"{"metadata":{"kept":1}}"#, r#"{"metadata":{"kept":1,"@R":[@E]}}"#)]
fn sync_creates_metadata_where_it_is_missing_or_null(#[case] params: &str, #[case] expected: &str) {
    let data = body(SYNCED);
    let mut target = targets(Some(params), None);
    sync_log_target(&data, &mut target);
    assert_eq!(target.litellm_params, Some(body(expected)));
    assert_eq!(target.model_call_params, None);
}

#[test]
fn sync_skips_a_none_target_and_wraps_a_single_entry_in_a_list() {
    let data = body(r#"{"metadata":{"@R":@E}}"#);
    let mut target = targets(None, Some("{}"));
    sync_log_target(&data, &mut target);
    assert_eq!(target.litellm_params, None);
    assert_eq!(target.model_call_params, Some(body(SYNCED)));
}

#[test]
fn sync_does_not_repeat_an_equal_entry() {
    let data = body(r#"{"metadata":{"@R":[@E,{"guardrail_name":"other"}]}}"#);
    let mut target = targets(
        Some(r#"{"metadata":{"@R":[{"guardrail_name":"other"}]}}"#),
        Some("{}"),
    );
    sync_log_target(&data, &mut target);
    sync_log_target(&data, &mut target);
    assert_eq!(
        target.litellm_params,
        Some(body(
            r#"{"metadata":{"@R":[{"guardrail_name":"other"},@E]}}"#
        ))
    );
    assert_eq!(
        target.model_call_params,
        Some(body(
            r#"{"metadata":{"@R":[@E,{"guardrail_name":"other"}]}}"#
        ))
    );
}

#[rstest]
#[case::no_bucket(r#"{"model":"m"}"#)]
#[case::bucket_without_entries(r#"{"metadata":{"a":1}}"#)]
#[case::null_entries(r#"{"metadata":{"@R":null}}"#)]
#[case::empty_entries(r#"{"metadata":{"@R":[]}}"#)]
#[case::bucket_not_an_object(r#"{"metadata":"text"}"#)]
fn sync_leaves_everything_alone_without_entries(#[case] before: &str) {
    let data = body(before);
    let mut target = targets(Some("{}"), Some("{}"));
    sync_log_target(&data, &mut target);
    assert_eq!(data, body(before));
    assert_eq!(target, targets(Some("{}"), Some("{}")));
}

#[test]
fn sync_reads_the_litellm_metadata_bucket_when_it_exists() {
    let data = body(
        r#"{"litellm_metadata":{"@R":[@E]},"metadata":{"@R":[{"guardrail_name":"ignored"}]}}"#,
    );
    let mut target = targets(Some("{}"), None);
    sync_log_target(&data, &mut target);
    assert_eq!(target.litellm_params, Some(body(SYNCED)));
}

#[rstest]
#[case::text_metadata(r#"{"metadata":"text"}"#)]
#[case::text_records(r#"{"metadata":{"@R":"text"}}"#)]
fn sync_skips_a_target_whose_metadata_has_the_wrong_shape(#[case] params: &str) {
    let data = body(SYNCED);
    let mut target = targets(Some(params), Some("{}"));
    sync_log_target(&data, &mut target);
    assert_eq!(target.litellm_params, Some(body(params)));
    assert_eq!(target.model_call_params, Some(body(SYNCED)));
}

#[rstest]
#[case::no_response(None, "{}")]
#[case::null_response(Some("null"), "{}")]
#[case::text(Some(r#""blocked by policy""#), r#""blocked by policy""#)]
#[case::list(Some("[1,2]"), "[1,2]")]
fn summary_gives_an_empty_object_without_a_response_and_keeps_other_non_objects(
    #[case] given: Option<&str>,
    #[case] expected: &str,
) {
    let baseline = body(r#"{"texts":["a"]}"#);
    let response = given.map(gv);
    let summary = summarize_guardrail_response(response.as_ref(), Some(&baseline), None);
    assert_eq!(summary, gv(expected));
}

#[test]
fn summary_keeps_an_object_response_when_there_is_no_baseline() {
    let given = gv(r#"{"texts":["secret prompt"]}"#);
    assert_eq!(
        summarize_guardrail_response(Some(&given), None, None),
        given
    );
}

#[rstest]
#[case::equal(r#"{"texts":["a"]}"#, r#"{"texts":["a"]}"#, "allow")]
#[case::changed(r#"{"texts":["a"]}"#, r#"{"texts":["b"]}"#, "mask")]
#[case::added_key(r#"{"texts":["a"]}"#, r#"{"texts":["a"],"images":["x"]}"#, "mask")]
#[case::removed_key(r#"{"texts":["a"],"images":["x"]}"#, r#"{"texts":["a"]}"#, "mask")]
#[case::missing_in_response_equals_null(r#"{"images":null}"#, "{}", "allow")]
#[case::missing_in_baseline_equals_null("{}", r#"{"images":null}"#, "allow")]
#[case::both_empty("{}", "{}", "allow")]
#[case::holdback_changed_alone(
    r#"{"texts":["a"],"stream_holdback_chars":[1]}"#,
    r#"{"texts":["a"],"stream_holdback_chars":[5]}"#,
    "allow"
)]
#[case::holdback_added_alone(
    r#"{"texts":["a"]}"#,
    r#"{"texts":["a"],"stream_holdback_chars":[5]}"#,
    "allow"
)]
#[case::holdback_with_a_content_change(
    r#"{"texts":["a"],"stream_holdback_chars":[1]}"#,
    r#"{"texts":["b"],"stream_holdback_chars":[1]}"#,
    "mask"
)]
fn summary_compares_baseline_and_response(
    #[case] baseline: &str,
    #[case] given: &str,
    #[case] expected: &str,
) {
    let summary = summarize_guardrail_response(Some(&gv(given)), Some(&body(baseline)), None);
    assert_eq!(summary, GuardrailValue::Text(expected.to_owned()));
}

#[rstest]
#[case::ignores_added_keys(
    Some(Event::PreCall),
    r#"{"messages":["a"],"model":"m","metadata":{"x":1}}"#,
    "allow"
)]
#[case::sees_changed_content(Some(Event::PreCall), r#"{"messages":["b"],"model":"m"}"#, "mask")]
#[case::sees_removed_content(Some(Event::PreCall), r#"{"model":"m"}"#, "mask")]
#[case::other_events_compare_everything(
    Some(Event::PostCall),
    r#"{"messages":["a"],"model":"m"}"#,
    "mask"
)]
#[case::no_event_compares_everything(None, r#"{"messages":["a"],"model":"m"}"#, "mask")]
fn summary_narrows_a_pre_call_response_to_content_keys(
    #[case] event_type: Option<Event>,
    #[case] given: &str,
    #[case] expected: &str,
) {
    let baseline = body(r#"{"messages":["a"]}"#);
    let summary = summarize_guardrail_response(Some(&gv(given)), Some(&baseline), event_type);
    assert_eq!(summary, GuardrailValue::Text(expected.to_owned()));
}

#[rstest]
#[case::match_null_dropped(r#"{"match":null,"k":1}"#, r#"{"k":1}"#)]
#[case::match_text_redacted(r#"{"match":"x"}"#, r#"{"match":"[REDACTED]"}"#)]
#[case::regex_redacted(
    r#"{"regex":"p.*","name":"n"}"#,
    r#"{"regex":"[REDACTED]","name":"n"}"#
)]
#[case::nested_in_list(r#"[{"match":null},{"match":"z"}]"#, r#"[{},{"match":"[REDACTED]"}]"#)]
#[case::null_list_items_dropped(r#"{"items":[null,1,null]}"#, r#"{"items":[1]}"#)]
#[case::text_stays_text(r#""Guardrail said no""#, r#""Guardrail said no""#)]
#[case::null_stays_null("null", "null")]
fn sanitize_drops_nulls_before_redacting_match(#[case] given: &str, #[case] expected: &str) {
    assert_eq!(
        sanitize_guardrail_response(gv(given)).unwrap(),
        gv(expected)
    );
}

#[rstest]
#[case::object_level_0(0, r#"{"gone":null,"kept":1}"#, r#"{"kept":1}"#)]
#[case::object_level_19(19, r#"{"gone":null,"kept":1}"#, r#"{"kept":1}"#)]
#[case::object_level_20(20, r#"{"gone":null,"kept":1}"#, r#"{"gone":null,"kept":1}"#)]
#[case::object_level_21(21, r#"{"gone":null,"kept":1}"#, r#"{"gone":null,"kept":1}"#)]
#[case::list_level_19(19, "[null,1]", "[1]")]
#[case::list_level_20(20, "[null,1]", "[null,1]")]
fn sanitize_drops_nulls_only_above_level_20(
    #[case] level: usize,
    #[case] given: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        drop_nulls(nested(level, gv(given)), MAX_NULL_DROP_DEPTH),
        nested(level, gv(expected))
    );
}

#[rstest]
#[case::root_object(r#"{"secret_fields":{"a":"b"},"x":1}"#, false)]
#[case::object_in_root_list(r#"[{"secret_fields":{"a":"b"},"x":1},"text"]"#, false)]
#[case::deeper_object(r#"{"inner":{"secret_fields":{"a":"b"}}}"#, true)]
#[case::object_deeper_in_list(r#"[{"inner":{"secret_fields":{"a":"b"}}}]"#, true)]
#[case::list_in_root_list(r#"[[{"secret_fields":{"a":"b"}}]]"#, true)]
fn sanitize_removes_secret_fields_only_at_the_root(#[case] given: &str, #[case] stays: bool) {
    let sanitized = sanitize_guardrail_response(gv(given)).unwrap();
    assert_eq!(
        serde_json::to_string(&sanitized)
            .unwrap()
            .contains("secret_fields"),
        stays
    );
}

#[test]
fn sanitize_masks_credentials_in_objects_and_lists_and_keeps_other_fields() {
    let given = r#"{"detail":{"authorization":"Bearer sk-abcdefghijkl","note":"visible"},
        "evidence":[{"api_key":"sk-abcdefghijkl","text":"fine"}]}"#;
    let sanitized = sanitize_guardrail_response(gv(given)).unwrap();
    assert_eq!(
        field(&sanitized, "detail"),
        gv(r#"{"authorization":"@M","note":"visible"}"#)
    );
    let evidence = serde_json::to_string(&field(&sanitized, "evidence")).unwrap();
    assert!(!evidence.contains("sk-abcdefghijkl"));
    assert!(evidence.contains(r#""text":"fine""#));
}

#[test]
fn sanitize_keeps_floats_as_numbers() {
    let given = gv(r#"{"score":0.75,"scores":[0.5,1.25]}"#);
    assert_eq!(
        plain(sanitize_guardrail_response(given.clone()).unwrap()),
        given
    );
}

const PRE_BASELINE: &str = r#"{"messages":[{"role":"user","content":"hi"}],"tools":[]}"#;
const OWN_DATA: &str = r#"{"metadata":{"@R":[{"guardrail_name":"own"}]}}"#;

#[rstest]
#[case::pre_call(Hook::PreCall, true, Some(Event::PreCall), Some(PRE_BASELINE))]
#[case::moderation(Hook::Moderation, true, Some(Event::DuringCall), None)]
#[case::post_call(Hook::PostCallSuccess, true, Some(Event::PostCall), None)]
#[case::streaming(Hook::PostCallStreaming, true, Some(Event::PostCall), None)]
#[case::iterator(Hook::StreamingIterator, true, None, None)]
#[case::apply(
    Hook::ApplyGuardrail,
    true,
    None,
    Some(r#"{"texts":["hi"],"model":"m"}"#)
)]
#[case::apply_without_inputs(Hook::ApplyGuardrail, false, None, None)]
fn invocation_begin_gives_each_hook_its_event_type_and_baseline(
    #[case] hook: Hook,
    #[case] with_inputs: bool,
    #[case] event_type: Option<Event>,
    #[case] baseline: Option<&str>,
) {
    let data = body(
        r#"{"messages":[{"role":"user","content":"hi"}],"tools":[],"model":"gpt","metadata":{"a":1}}"#,
    );
    let inputs: GuardrailApiInputs =
        serde_json::from_str(r#"{"texts":["hi"],"model":"m"}"#).unwrap();
    let invocation =
        GuardrailInvocation::begin(hook, &data, with_inputs.then_some(&inputs), at(0)).unwrap();
    assert!(!invocation.self_recorded);
    assert_eq!(invocation.event_type, event_type);
    assert_eq!(invocation.baseline, baseline.map(body));
}

#[rstest]
#[case::unchanged("hi", "other", "allow")]
#[case::changed("hidden", "m", "mask")]
fn invocation_finish_success_records_the_summary_with_epoch_times(
    #[case] content: &str,
    #[case] model: &str,
    #[case] verdict: &str,
) {
    let mut run = named_run(r#"{"messages":[{"role":"user","content":"hi"}],"model":"m"}"#);
    let invocation = run.begin(Hook::PreCall, 100_250);
    let returned =
        format!(r#"{{"messages":[{{"role":"user","content":"{content}"}}],"model":"{model}"}}"#);
    run.succeed(invocation, Some(&returned), 102_750);
    let overrides = format!(
        r#"{{"guardrail_name":"pii","guardrail_mode":"pre_call","guardrail_response":"{verdict}",
        "start_time":100.25,"end_time":102.75,"duration":2.5}}"#
    );
    assert_eq!(
        stored(&run.data, "metadata"),
        vec![expected_entry(&overrides)]
    );
}

#[rstest]
#[case::event_type(Hook::PreCall, "pre_call")]
#[case::during_call(Hook::Moderation, "during_call")]
#[case::configured_hook_for_iterator(Hook::StreamingIterator, "post_call")]
#[case::configured_hook_for_apply(Hook::ApplyGuardrail, "post_call")]
fn invocation_record_mode_is_the_event_type_or_the_configured_hook(
    #[case] hook: Hook,
    #[case] mode: &str,
) {
    let mut run = named_run("{}");
    let invocation = run.begin(hook, 0);
    run.succeed(invocation, Some(r#""blocked""#), 0);
    assert_eq!(
        field(&stored(&run.data, "metadata")[0], "guardrail_mode"),
        GuardrailValue::Text(mode.to_owned())
    );
}

#[rstest]
#[case::records(false, false, 1, 1)]
#[case::self_recorded(true, false, 0, 0)]
#[case::records_own_information(false, true, 0, 1)]
#[case::both(true, true, 0, 0)]
fn invocation_finish_skips_what_the_guardrail_recorded_itself(
    #[case] self_recorded: bool,
    #[case] records_own_information: bool,
    #[case] after_success: usize,
    #[case] after_failure: usize,
) {
    let mut succeeded = own_info_run(records_own_information, "{}");
    let mut invocation = succeeded.begin(Hook::PostCallSuccess, 0);
    invocation.self_recorded = self_recorded;
    succeeded.succeed(invocation.clone(), None, 0);
    assert_eq!(stored_count(&succeeded.data), after_success);
    let mut failed = own_info_run(records_own_information, "{}");
    let failure = GuardrailFailure::Other("boom".to_owned());
    assert_eq!(failed.fail(invocation, &failure, 0), failure);
    assert_eq!(stored_count(&failed.data), after_failure);
}

fn http(status: u16, detail: &str) -> GuardrailFailure {
    GuardrailFailure::Http {
        status,
        detail: detail.to_owned(),
    }
}

fn raised() -> GuardrailFailure {
    GuardrailFailure::GuardrailRaised {
        guardrail_name: Some("pii".to_owned()),
        message: "nope".to_owned(),
        wrap_default_message: false,
        status_code: 400,
        blocked_content: true,
    }
}

#[rstest]
#[case::http_400(http(400, "blocked"), "guardrail_intervened", "blocked")]
#[case::http_403(http(403, "denied"), "guardrail_intervened", "denied")]
#[case::http_422(http(422, "bad"), "guardrail_intervened", "bad")]
#[case::http_401(http(401, "auth"), "guardrail_failed_to_respond", "auth")]
#[case::http_500(http(500, "down"), "guardrail_failed_to_respond", "down")]
#[case::other(GuardrailFailure::Other("boom".to_owned()), "guardrail_failed_to_respond", "boom")]
#[case::raised(raised(), "guardrail_intervened", "nope")]
fn invocation_finish_failure_stores_the_status_and_text_and_returns_the_failure(
    #[case] failure: GuardrailFailure,
    #[case] status: &str,
    #[case] message: &str,
) {
    let mut run = named_run("{}");
    let invocation = run.begin(Hook::Moderation, 1_000);
    assert_eq!(run.fail(invocation, &failure, 1_500), failure);
    let overrides = format!(
        r#"{{"guardrail_name":"pii","guardrail_mode":"during_call","guardrail_response":"{message}",
        "guardrail_status":"{status}","start_time":1.0,"end_time":1.5,"duration":0.5}}"#
    );
    assert_eq!(
        stored(&run.data, "metadata"),
        vec![expected_entry(&overrides)]
    );
}

#[test]
fn invocation_finish_failure_stores_deny_when_the_guardrail_asks_for_it() {
    let mut run = named_run("{}");
    run.base.settings.switches.deny_text_on_error = true;
    let invocation = run.begin(Hook::PreCall, 0);
    run.fail(invocation, &http(400, "secret detail"), 0);
    let entry = &stored(&run.data, "metadata")[0];
    assert_eq!(field(entry, "guardrail_response"), gv(r#""deny""#));
    assert_eq!(
        field(entry, "guardrail_status"),
        gv(r#""guardrail_intervened""#)
    );
}

fn assert_synced_with_own_entry(run: &Run) {
    let synced = body(OWN_DATA);
    assert_eq!(run.target.litellm_params, Some(synced.clone()));
    assert_eq!(run.target.model_call_params, Some(synced));
}

#[test]
fn invocation_finish_syncs_the_log_target_when_it_records_nothing() {
    let mut succeeded = own_info_run(true, OWN_DATA);
    let finished = succeeded.begin(Hook::PostCallSuccess, 0);
    succeeded.succeed(finished, None, 0);
    assert_synced_with_own_entry(&succeeded);
    let mut failed = own_info_run(false, OWN_DATA);
    let mut self_recorded = failed.begin(Hook::PostCallSuccess, 0);
    self_recorded.self_recorded = true;
    failed.fail(
        self_recorded,
        &GuardrailFailure::Other("boom".to_owned()),
        0,
    );
    assert_synced_with_own_entry(&failed);
}

#[test]
fn invocation_record_own_stores_one_entry_and_marks_the_run() {
    let mut run = named_run("{}");
    let mut invocation = run.begin(Hook::ApplyGuardrail, 0);
    let input = GuardrailRecordInput {
        guardrail_provider: Some("bedrock".to_owned()),
        ..success(r#"{"verdict":"ok"}"#)
    };
    invocation
        .record_own(&mut run.context(&DiscardSink), input)
        .unwrap();
    assert!(invocation.self_recorded);
    run.succeed(invocation, Some(r#"{"texts":[]}"#), 0);
    let entries = stored(&run.data, "metadata");
    assert_eq!(entries.len(), 1);
    assert_eq!(field(&entries[0], "guardrail_provider"), gv(r#""bedrock""#));
}

#[test]
fn invocation_record_own_stops_a_failure_from_being_recorded_twice() {
    let mut run = named_run("{}");
    let mut invocation = run.begin(Hook::ApplyGuardrail, 0);
    invocation
        .record_own(&mut run.context(&DiscardSink), success("{}"))
        .unwrap();
    run.fail(invocation, &GuardrailFailure::Other("boom".to_owned()), 0);
    assert_eq!(stored_count(&run.data), 1);
}

#[test]
fn invocation_a_clock_that_steps_back_gives_a_zero_duration() {
    let mut run = named_run("{}");
    let invocation = run.begin(Hook::PostCallSuccess, 5_000);
    run.succeed(invocation, None, 4_000);
    let entry = &stored(&run.data, "metadata")[0];
    assert_eq!(field(entry, "start_time"), gv("5.0"));
    assert_eq!(field(entry, "end_time"), gv("4.0"));
    assert_eq!(field(entry, "duration"), gv("0.0"));
}

fn cache() -> DualLocalCache {
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
        emit: Arc::new(|_event| {}),
    };
    DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    )
}

fn lookup(
    call_type: &str,
    data: &str,
) -> Result<Option<Vec<GuardrailValue>>, GuardrailRecordError> {
    messages_for_call_type(call_type, Some(&body(data)), &cache())
}

#[test]
fn messages_are_none_without_data() {
    assert!(
        messages_for_call_type("completion", None, &cache())
            .unwrap()
            .is_none()
    );
}

const CHAT_MESSAGES: &str = r#"[{"role":"user","content":"hi"}]"#;
const CHAT_DATA: &str = r#"{"messages":[{"role":"user","content":"hi"}],"input":"ignored"}"#;
const RESPONSES_DATA: &str =
    r#"{"input":"hi","instructions":"follow","messages":[{"role":"user","content":"ignored"}]}"#;
const RESPONSES_MESSAGES: &str =
    r#"[{"role":"system","content":"follow"},{"role":"user","content":"hi"}]"#;

#[rstest]
#[case::completion("completion", CHAT_DATA, CHAT_MESSAGES)]
#[case::acompletion("acompletion", CHAT_DATA, CHAT_MESSAGES)]
#[case::anthropic_messages("anthropic_messages", CHAT_DATA, CHAT_MESSAGES)]
#[case::call_mcp_tool("call_mcp_tool", CHAT_DATA, CHAT_MESSAGES)]
#[case::list_mcp_tools("list_mcp_tools", CHAT_DATA, CHAT_MESSAGES)]
#[case::responses("responses", RESPONSES_DATA, RESPONSES_MESSAGES)]
#[case::aresponses("aresponses", RESPONSES_DATA, RESPONSES_MESSAGES)]
fn messages_returns_the_messages_of_chat_like_call_types(
    #[case] call_type: &str,
    #[case] data: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        lookup(call_type, data).unwrap().map(list),
        Some(gv(expected))
    );
}

#[test]
fn messages_refuse_a_chat_request_whose_messages_are_not_a_list() {
    let error = lookup("acompletion", r#"{"messages":"hi"}"#).unwrap_err();
    assert!(matches!(error, GuardrailRecordError::MessagesShape { .. }));
}

#[test]
fn messages_refuse_a_responses_input_of_the_wrong_shape() {
    let error = lookup("responses", r#"{"input":5}"#).unwrap_err();
    assert!(matches!(error, GuardrailRecordError::ResponsesInput(_)));
}

const CHAT_AND_INPUT: &str = r#"{"messages":[{"role":"user","content":"hi"}],"input":"hi"}"#;

#[rstest]
#[case::chat_without_messages("completion", "{}")]
#[case::chat_with_null_messages("completion", r#"{"messages":null}"#)]
#[case::responses_without_input("responses", r#"{"instructions":"follow"}"#)]
#[case::responses_with_null_input("aresponses", r#"{"input":null,"instructions":"follow"}"#)]
#[case::embedding("embedding", CHAT_AND_INPUT)]
#[case::text_completion("text_completion", CHAT_AND_INPUT)]
#[case::image_generation("image_generation", CHAT_AND_INPUT)]
#[case::empty_call_type("", CHAT_AND_INPUT)]
fn messages_are_none_when_there_is_nothing_to_read(#[case] call_type: &str, #[case] data: &str) {
    assert!(lookup(call_type, data).unwrap().is_none());
}
