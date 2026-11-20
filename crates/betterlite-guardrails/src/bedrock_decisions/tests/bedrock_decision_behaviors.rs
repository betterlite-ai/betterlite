use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{
    DiscardSink, as_json, base, call_log, catalog, context, empty_request_data, json, record_json,
    response,
};
use crate::bedrock_decisions::assessments::{
    blocked_assessments, should_raise_blocked, violation_category_names,
};
use crate::bedrock_decisions::exceptions::{
    BlockedGuardrail, blocked_guardrail_failure, response_status,
};
use crate::bedrock_decisions::logging::{AttemptLog, log_attempt, log_failure, log_success};
use crate::bedrock_decisions::tracing::{Pricing, tracing_detail};
use crate::bedrock_records::Usage;
use crate::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use crate::guardrail_records::record::{GuardrailStatus, GuardrailTracingDetail};

#[rstest]
#[case(
    r#"{"action":"GUARDRAIL_INTERVENED","assessments":[{"topicPolicy":{"topics":[{"action":"BLOCKED"}]}}]}"#,
    true
)]
#[case(
    r#"{"action":"GUARDRAIL_INTERVENED","assessments":[{"topicPolicy":{"topics":[{"action":"ANONYMIZED"}]}}]}"#,
    false
)]
#[case(
    r#"{"action":"GUARDRAIL_INTERVENED","assessments":[{"topicPolicy":{"topics":[{"action":"NONE"}]}}]}"#,
    false
)]
#[case(r#"{"action":"GUARDRAIL_INTERVENED"}"#, false)]
#[case(r#"{"action":"GUARDRAIL_INTERVENED","assessments":[]}"#, false)]
#[case(
    r#"{"action":"NONE","assessments":[{"topicPolicy":{"topics":[{"action":"BLOCKED"}]}}]}"#,
    false
)]
#[case(
    r#"{"action":"intervened","assessments":[{"topicPolicy":{"topics":[{"action":"BLOCKED"}]}}]}"#,
    false
)]
#[case(
    r#"{"action":"GUARDRAIL_INTERVENED","assessments":[{"contentPolicy":{"filters":[{"action":"BLOCKED"}]}},{"sensitiveInformationPolicy":{"regexes":[{"action":"BLOCKED"}]}}]}"#,
    true
)]
fn should_raise_only_for_intervened_and_blocked(#[case] wire: &str, #[case] expected: bool) {
    assert_eq!(should_raise_blocked(&response(wire)), expected);
}

#[rstest]
#[case(
    r#"{"assessments":[{"topicPolicy":{"topics":[{"name":"finance","type":"DENY","action":"BLOCKED"},{"name":"medical","type":"DENY","action":"ANONYMIZED"}]}}]}"#,
    r#"[{"policy":"topicPolicy","matches":[{"category":"topics","name":"finance","type":"DENY","action":"BLOCKED"}]}]"#
)]
#[case(
    r#"{"assessments":[{"contentPolicy":{"filters":[{"type":"HATE","confidence":"HIGH","filterStrength":"HIGH","action":"BLOCKED"},{"type":"VIOLENCE","confidence":"LOW","filterStrength":"MEDIUM","action":"NONE"}]}}]}"#,
    r#"[{"policy":"contentPolicy","matches":[{"category":"filters","type":"HATE","confidence":"HIGH","filterStrength":"HIGH","action":"BLOCKED"}]}]"#
)]
#[case(
    r#"{"assessments":[{"wordPolicy":{"customWords":[{"match":"secret","action":"BLOCKED"},{"match":"fine","action":"NONE"}],"managedWordLists":[{"match":"profane","type":"PROFANITY","action":"BLOCKED"}]}}]}"#,
    r#"[{"policy":"wordPolicy","matches":[{"category":"customWords","match":"secret","action":"BLOCKED"},{"category":"managedWordLists","type":"PROFANITY","match":"profane","action":"BLOCKED"}]}]"#
)]
#[case(
    r#"{"assessments":[{"sensitiveInformationPolicy":{"piiEntities":[{"type":"EMAIL","match":"a@b.c","action":"BLOCKED"}],"regexes":[{"name":"account","regex":"\\d{4}","match":"1234","action":"BLOCKED"}]}}]}"#,
    r#"[{"policy":"sensitiveInformationPolicy","matches":[{"category":"piiEntities","type":"EMAIL","match":"a@b.c","action":"BLOCKED"},{"category":"regexes","name":"account","regex":"\\d{4}","match":"1234","action":"BLOCKED"}]}]"#
)]
#[case(
    r#"{"assessments":[{"contextualGroundingPolicy":{"filters":[{"type":"GROUNDING","threshold":0.75,"score":0.5,"action":"BLOCKED"},{"type":"RELEVANCE","threshold":0.5,"score":0.9,"action":"BLOCKED"}]}}]}"#,
    r#"[{"policy":"contextualGroundingPolicy","matches":[{"category":"filters","type":"GROUNDING","threshold":0.75,"score":0.5,"action":"BLOCKED"},{"category":"filters","type":"RELEVANCE","threshold":0.5,"score":0.9,"action":"BLOCKED"}]}]"#
)]
#[case(
    r#"{"assessments":[{"topicPolicy":{"topics":[{"action":"BLOCKED"}]}}]}"#,
    r#"[{"policy":"topicPolicy","matches":[{"category":"topics","name":null,"type":null,"action":"BLOCKED"}]}]"#
)]
#[case(
    r#"{"assessments":[{"topicPolicy":null,"contentPolicy":{},"wordPolicy":{"customWords":[]},"sensitiveInformationPolicy":{"piiEntities":null,"regexes":[]}}]}"#,
    "[]"
)]
fn blocked_assessments_emit_a_block_per_policy_with_matches(
    #[case] wire: &str,
    #[case] expected: &str,
) {
    let blocked = blocked_assessments(&response(wire));
    assert_eq!(as_json(&blocked), json(expected));
}

#[test]
fn blocked_assessments_keep_one_block_per_assessment_and_policy_order() {
    let wire = r#"{"assessments":[{"contentPolicy":{"filters":[{"type":"HATE","action":"BLOCKED"}]}},{"contentPolicy":{"filters":[{"type":"SEXUAL","action":"BLOCKED"}]},"wordPolicy":{"customWords":[{"match":"secret","action":"BLOCKED"}]}}]}"#;
    let blocked = blocked_assessments(&response(wire));
    let expected = json(
        r#"[{"policy":"contentPolicy","matches":[{"category":"filters","type":"HATE","confidence":null,"filterStrength":null,"action":"BLOCKED"}]},{"policy":"contentPolicy","matches":[{"category":"filters","type":"SEXUAL","confidence":null,"filterStrength":null,"action":"BLOCKED"}]},{"policy":"wordPolicy","matches":[{"category":"customWords","match":"secret","action":"BLOCKED"}]}]"#,
    );
    assert_eq!(as_json(&blocked), expected);
}

#[test]
fn violation_names_use_labels_and_skip_the_match_field() {
    let wire = r#"{"assessments":[{"topicPolicy":{"topics":[{"name":"finance","action":"BLOCKED"}]},"contentPolicy":{"filters":[{"type":"HATE","action":"BLOCKED"}]},"wordPolicy":{"customWords":[{"match":"secret","action":"BLOCKED"}],"managedWordLists":[{"match":"profane","type":"PROFANITY","action":"BLOCKED"}]},"sensitiveInformationPolicy":{"piiEntities":[{"type":"EMAIL","match":"a@b.c","action":"BLOCKED"}],"regexes":[{"name":"account","regex":"\\d{4}","match":"1234","action":"BLOCKED"}]},"contextualGroundingPolicy":{"filters":[{"type":"GROUNDING","action":"BLOCKED"}]}}]}"#;
    let names = violation_category_names(&response(wire));
    assert_eq!(
        names,
        vec![
            "finance",
            "HATE",
            "PROFANITY",
            "EMAIL",
            "account",
            "GROUNDING"
        ]
    );
}

#[test]
fn violation_names_skip_entries_without_a_usable_label() {
    let wire = r#"{"assessments":[{"sensitiveInformationPolicy":{"piiEntities":[{"match":"4111","action":"BLOCKED"}],"regexes":[{"name":"","regex":"\\d+","match":"4111","action":"BLOCKED"}]}}]}"#;
    assert_eq!(
        violation_category_names(&response(wire)),
        Vec::<String>::new()
    );
}

#[rstest]
#[case(
    200,
    r#"{"Output":{"__type":"ValidationException"}}"#,
    GuardrailStatus::GuardrailFailedToRespond
)]
#[case(
    200,
    r#"{"action":"GUARDRAIL_INTERVENED","assessments":[{"topicPolicy":{"topics":[{"action":"BLOCKED"}]}}]}"#,
    GuardrailStatus::GuardrailIntervened
)]
#[case(
    200,
    r#"{"action":"GUARDRAIL_INTERVENED","assessments":[{"topicPolicy":{"topics":[{"action":"ANONYMIZED"}]}}]}"#,
    GuardrailStatus::Success
)]
#[case(200, r#"{"action":"NONE"}"#, GuardrailStatus::Success)]
#[case(200, "not json", GuardrailStatus::Success)]
#[case(
    400,
    r#"{"Output":{"__type":"ValidationException"}}"#,
    GuardrailStatus::GuardrailFailedToRespond
)]
#[case(
    500,
    r#"{"action":"GUARDRAIL_INTERVENED","assessments":[{"topicPolicy":{"topics":[{"action":"BLOCKED"}]}}]}"#,
    GuardrailStatus::GuardrailFailedToRespond
)]
#[case(201, "{}", GuardrailStatus::GuardrailFailedToRespond)]
fn response_status_maps_http_status_and_body(
    #[case] status: u16,
    #[case] wire: &str,
    #[case] expected: GuardrailStatus,
) {
    assert_eq!(response_status(status, wire.as_bytes()), expected);
}

#[test]
fn tracing_detail_carries_categories_action_usage_and_cost() {
    let prices = catalog(&[(
        "bedrock/guardrails",
        r#"{"topicPolicyUnits":0.25,"wordPolicyUnits":0.5}"#,
    )]);
    let pricing = Pricing {
        catalog: &prices,
        aws_region_name: None,
    };
    let merged = response(
        r#"{"action":"GUARDRAIL_INTERVENED","usage":{"topicPolicyUnits":4,"wordPolicyUnits":null,"futureUnits":2},"assessments":[{"topicPolicy":{"topics":[{"name":"finance","action":"BLOCKED"}]}}]}"#,
    );
    let detail = tracing_detail(&merged, &pricing).expect("tracing detail resolves");
    let expected = json(
        r#"{"violation_categories":["finance"],"guardrail_action":"GUARDRAIL_INTERVENED","guardrail_usage":{"topicPolicyUnits":4,"futureUnits":2},"guardrail_cost":1.0,"guardrail_cost_by_unit":{"topicPolicyUnits":1.0,"futureUnits":null}}"#,
    );
    assert_eq!(as_json(&detail), expected);
}

#[test]
fn tracing_detail_omits_categories_and_the_action_when_absent() {
    let prices = catalog(&[("bedrock/guardrails", r#"{"wordPolicyUnits":0.5}"#)]);
    let pricing = Pricing {
        catalog: &prices,
        aws_region_name: None,
    };
    let merged = response(r#"{"usage":{"wordPolicyUnits":2},"outputs":[{"text":"ok"}]}"#);
    let detail = tracing_detail(&merged, &pricing).expect("tracing detail resolves");
    let expected = json(
        r#"{"guardrail_usage":{"wordPolicyUnits":2},"guardrail_cost":1.0,"guardrail_cost_by_unit":{"wordPolicyUnits":1.0}}"#,
    );
    assert_eq!(as_json(&detail), expected);
}

#[test]
fn tracing_detail_is_empty_without_an_integer_counter() {
    let prices = catalog(&[("bedrock/guardrails", r#"{"wordPolicyUnits":0.5}"#)]);
    let pricing = Pricing {
        catalog: &prices,
        aws_region_name: None,
    };
    let null_usage = response(r#"{"usage":{"wordPolicyUnits":null}}"#);
    assert_eq!(
        tracing_detail(&null_usage, &pricing).expect("tracing detail resolves"),
        GuardrailTracingDetail::default()
    );
    let absent = response(r#"{"action":"NONE"}"#);
    let detail = tracing_detail(&absent, &pricing).expect("tracing detail resolves");
    assert_eq!(as_json(&detail), json(r#"{"guardrail_action":"NONE"}"#));
}

#[test]
fn tracing_cost_prefers_the_regional_pricing_key() {
    let prices = catalog(&[
        (
            "bedrock/us-east-1/guardrails",
            r#"{"contentPolicyUnits":0.5}"#,
        ),
        ("bedrock/guardrails", r#"{"contentPolicyUnits":2}"#),
    ]);
    let pricing = Pricing {
        catalog: &prices,
        aws_region_name: Some("us-east-1"),
    };
    let merged = response(r#"{"usage":{"contentPolicyUnits":2}}"#);
    let detail = tracing_detail(&merged, &pricing).expect("tracing detail resolves");
    let expected = json(
        r#"{"guardrail_usage":{"contentPolicyUnits":2},"guardrail_cost":1.0,"guardrail_cost_by_unit":{"contentPolicyUnits":1.0}}"#,
    );
    assert_eq!(as_json(&detail), expected);
}

#[test]
fn http_exception_reports_output_text_and_redacted_assessments() {
    let merged = response(
        r#"{"outputs":[{"text":"Request blocked: "},{"text":"secret"},{"text":""}],"assessments":[{"wordPolicy":{"customWords":[{"match":"secret","action":"BLOCKED"}]},"sensitiveInformationPolicy":{"regexes":[{"name":"account","regex":"\\d{4}","match":"1234","action":"BLOCKED"}]}}]}"#,
    );
    let request_data = BTreeMap::from([(
        "model".to_owned(),
        GuardrailValue::Text("gpt-4o".to_owned()),
    )]);
    let guardrail = BlockedGuardrail {
        identifier: Some("gr-1"),
        version: Some("3"),
        name: Some("gr-name"),
        disable_exception_on_block: false,
    };
    let failure = blocked_guardrail_failure(&merged, &request_data, &guardrail);
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("a blocked guardrail without disable_exception_on_block is an HTTP object failure");
    };
    assert_eq!(status, 400);
    let expected = json(
        r#"{"error":"Violated guardrail policy","bedrock_guardrail_response":"Request blocked: secret","guardrailIdentifier":"gr-1","guardrailVersion":"3","assessments":[{"policy":"wordPolicy","matches":[{"category":"customWords","match":"[REDACTED]","action":"BLOCKED"}]},{"policy":"sensitiveInformationPolicy","matches":[{"category":"regexes","name":"account","regex":"[REDACTED]","match":"[REDACTED]","action":"BLOCKED"}]}]}"#,
    );
    assert_eq!(as_json(&detail), expected);
}

#[test]
fn http_exception_omits_unknown_and_empty_guardrail_identity() {
    let merged = response(r#"{"outputs":[{"text":"blocked"}]}"#);
    let request_data = empty_request_data();
    let missing = BlockedGuardrail {
        identifier: None,
        version: None,
        name: None,
        disable_exception_on_block: false,
    };
    let failure = blocked_guardrail_failure(&merged, &request_data, &missing);
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("a blocked guardrail without disable_exception_on_block is an HTTP object failure");
    };
    assert_eq!(status, 400);
    let expected =
        json(r#"{"error":"Violated guardrail policy","bedrock_guardrail_response":"blocked"}"#);
    assert_eq!(as_json(&detail), expected);
    let empty = BlockedGuardrail {
        identifier: Some(""),
        version: Some(""),
        name: Some("gr-name"),
        disable_exception_on_block: false,
    };
    let empty_failure = blocked_guardrail_failure(&merged, &request_data, &empty);
    let GuardrailFailure::HttpObject {
        detail: empty_detail,
        ..
    } = empty_failure
    else {
        panic!("an empty identifier or version is left out of the detail");
    };
    assert_eq!(as_json(&empty_detail), expected);
}

#[test]
fn modify_response_exception_carries_the_request() {
    let merged = response(r#"{"outputs":[{"text":"blocked"}]}"#);
    let request_data = BTreeMap::from([(
        "model".to_owned(),
        GuardrailValue::Text("bedrock/claude".to_owned()),
    )]);
    let guardrail = BlockedGuardrail {
        identifier: None,
        version: None,
        name: Some("gr-name"),
        disable_exception_on_block: true,
    };
    let failure = blocked_guardrail_failure(&merged, &request_data, &guardrail);
    let expected = GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
        message: "blocked".to_owned(),
        model: "bedrock/claude".to_owned(),
        request_data,
        guardrail_name: Some("gr-name".to_owned()),
        detection_info: BTreeMap::new(),
        original_response: None,
    }));
    assert_eq!(failure, expected);
}

#[test]
fn modify_response_exception_defaults_the_model_name() {
    let merged = response(r#"{"outputs":null}"#);
    let guardrail = BlockedGuardrail {
        identifier: None,
        version: None,
        name: None,
        disable_exception_on_block: true,
    };
    let failure = blocked_guardrail_failure(&merged, &empty_request_data(), &guardrail);
    let GuardrailFailure::ModifyResponse(found) = failure else {
        panic!("disable_exception_on_block is a modify response failure");
    };
    assert_eq!(found.message, "");
    assert_eq!(found.model, "bedrock-guardrail");
}

#[test]
fn log_attempt_sums_completed_and_blocking_usage() {
    let prices = catalog(&[("bedrock/guardrails", r#"{"topicPolicyUnits":0.25}"#)]);
    let wire = r#"{"action":"GUARDRAIL_INTERVENED","usage":{"topicPolicyUnits":2},"assessments":[{"topicPolicy":{"topics":[{"name":"finance","action":"BLOCKED"}]}}]}"#;
    let blocking = response(wire);
    let completed = [Usage(BTreeMap::from([(
        "topicPolicyUnits".to_owned(),
        Some(1),
    )]))];
    let call = call_log(&prices);
    let attempt = AttemptLog {
        status_code: 200,
        body: wire.as_bytes(),
        json_response: &blocking,
        completed_chunk_usages: &completed,
    };
    let guardrail_base = base(Some("gr-name"));
    let mut request_data = empty_request_data();
    let sink = DiscardSink;
    let mut record_context = context(&guardrail_base, &mut request_data, &sink);
    log_attempt(&mut record_context, &call, &attempt).expect("the attempt record is written");
    let expected = json(
        r#"{"guardrail_name":"gr-name","guardrail_provider":"bedrock","guardrail_mode":"during_call","guardrail_response":{"action":"GUARDRAIL_INTERVENED","usage":{"topicPolicyUnits":3},"assessments":[{"topicPolicy":{"topics":[{"name":"finance","action":"BLOCKED"}]}}]},"guardrail_status":"guardrail_intervened","start_time":100.25,"end_time":102.75,"duration":2.5,"masked_entity_count":null,"violation_categories":["finance"],"guardrail_action":"GUARDRAIL_INTERVENED","guardrail_usage":{"topicPolicyUnits":3},"guardrail_cost":0.75,"guardrail_cost_by_unit":{"topicPolicyUnits":0.75}}"#,
    );
    assert_eq!(record_json(&request_data), expected);
}

#[test]
fn log_attempt_without_completed_chunks_keeps_the_blocking_usage() {
    let prices = catalog(&[("bedrock/guardrails", r#"{"topicPolicyUnits":0.25}"#)]);
    let wire = r#"{"action":"GUARDRAIL_INTERVENED","usage":{"topicPolicyUnits":2},"assessments":[{"topicPolicy":{"topics":[{"name":"finance","action":"BLOCKED"}]}}]}"#;
    let blocking = response(wire);
    let call = call_log(&prices);
    let attempt = AttemptLog {
        status_code: 200,
        body: wire.as_bytes(),
        json_response: &blocking,
        completed_chunk_usages: &[],
    };
    let guardrail_base = base(Some("gr-name"));
    let mut request_data = empty_request_data();
    let sink = DiscardSink;
    let mut record_context = context(&guardrail_base, &mut request_data, &sink);
    log_attempt(&mut record_context, &call, &attempt).expect("the attempt record is written");
    let expected = json(
        r#"{"guardrail_name":"gr-name","guardrail_provider":"bedrock","guardrail_mode":"during_call","guardrail_response":{"action":"GUARDRAIL_INTERVENED","usage":{"topicPolicyUnits":2},"assessments":[{"topicPolicy":{"topics":[{"name":"finance","action":"BLOCKED"}]}}]},"guardrail_status":"guardrail_intervened","start_time":100.25,"end_time":102.75,"duration":2.5,"masked_entity_count":null,"violation_categories":["finance"],"guardrail_action":"GUARDRAIL_INTERVENED","guardrail_usage":{"topicPolicyUnits":2},"guardrail_cost":0.5,"guardrail_cost_by_unit":{"topicPolicyUnits":0.5}}"#,
    );
    assert_eq!(record_json(&request_data), expected);
}

#[test]
fn log_attempt_without_blocking_usage_sums_only_the_chunks() {
    let prices = catalog(&[("bedrock/guardrails", r#"{"topicPolicyUnits":0.25}"#)]);
    let wire = r#"{"action":"NONE","assessments":[{"topicPolicy":{"topics":[{"name":"finance","action":"ANONYMIZED"}]}}]}"#;
    let blocking = response(wire);
    let completed = [Usage(BTreeMap::from([(
        "topicPolicyUnits".to_owned(),
        Some(2),
    )]))];
    let call = call_log(&prices);
    let attempt = AttemptLog {
        status_code: 200,
        body: wire.as_bytes(),
        json_response: &blocking,
        completed_chunk_usages: &completed,
    };
    let guardrail_base = base(Some("gr-name"));
    let mut request_data = empty_request_data();
    let sink = DiscardSink;
    let mut record_context = context(&guardrail_base, &mut request_data, &sink);
    log_attempt(&mut record_context, &call, &attempt).expect("the attempt record is written");
    let expected = json(
        r#"{"guardrail_name":"gr-name","guardrail_provider":"bedrock","guardrail_mode":"during_call","guardrail_response":{"action":"NONE","usage":{"topicPolicyUnits":2},"assessments":[{"topicPolicy":{"topics":[{"name":"finance","action":"ANONYMIZED"}]}}]},"guardrail_status":"success","start_time":100.25,"end_time":102.75,"duration":2.5,"masked_entity_count":null,"guardrail_action":"NONE","guardrail_usage":{"topicPolicyUnits":2},"guardrail_cost":0.5,"guardrail_cost_by_unit":{"topicPolicyUnits":0.5}}"#,
    );
    assert_eq!(record_json(&request_data), expected);
}

#[test]
fn log_success_preserves_the_exception_marker() {
    let prices = catalog(&[("bedrock/guardrails", r#"{"topicPolicyUnits":0.25}"#)]);
    let merged = response(
        r#"{"Output":{"__type":"ValidationException"},"outputs":[{"text":"masked"}],"usage":{"topicPolicyUnits":1}}"#,
    );
    let call = call_log(&prices);
    let guardrail_base = base(Some("gr-name"));
    let mut request_data = empty_request_data();
    let sink = DiscardSink;
    let mut record_context = context(&guardrail_base, &mut request_data, &sink);
    log_success(&mut record_context, &call, &merged).expect("the success record is written");
    let expected = json(
        r#"{"guardrail_name":"gr-name","guardrail_provider":"bedrock","guardrail_mode":"during_call","guardrail_response":{"usage":{"topicPolicyUnits":1},"outputs":[{"text":"masked"}],"Output":{"__type":"ValidationException"}},"guardrail_status":"guardrail_failed_to_respond","start_time":100.25,"end_time":102.75,"duration":2.5,"masked_entity_count":null,"guardrail_usage":{"topicPolicyUnits":1},"guardrail_cost":0.25,"guardrail_cost_by_unit":{"topicPolicyUnits":0.25}}"#,
    );
    assert_eq!(record_json(&request_data), expected);
}

#[test]
fn log_failure_wraps_the_error_and_bills_completed_chunks() {
    let prices = catalog(&[("bedrock/guardrails", r#"{"contentPolicyUnits":0.5}"#)]);
    let completed = [
        Usage(BTreeMap::from([("contentPolicyUnits".to_owned(), Some(1))])),
        Usage(BTreeMap::from([("contentPolicyUnits".to_owned(), Some(2))])),
    ];
    let call = call_log(&prices);
    let guardrail_base = base(Some("gr-name"));
    let mut request_data = empty_request_data();
    let sink = DiscardSink;
    let mut record_context = context(&guardrail_base, &mut request_data, &sink);
    log_failure(&mut record_context, &call, "upstream timeout", &completed)
        .expect("the failure record is written");
    let expected = json(
        r#"{"guardrail_name":"gr-name","guardrail_provider":"bedrock","guardrail_mode":"during_call","guardrail_response":{"error":"upstream timeout","usage":{"contentPolicyUnits":3}},"guardrail_status":"guardrail_failed_to_respond","start_time":100.25,"end_time":102.75,"duration":2.5,"masked_entity_count":null,"guardrail_usage":{"contentPolicyUnits":3},"guardrail_cost":1.5,"guardrail_cost_by_unit":{"contentPolicyUnits":1.5}}"#,
    );
    assert_eq!(record_json(&request_data), expected);
}

#[test]
fn log_failure_without_completed_chunks_writes_only_the_error() {
    let prices = catalog(&[("bedrock/guardrails", r#"{"contentPolicyUnits":0.5}"#)]);
    let call = call_log(&prices);
    let guardrail_base = base(Some("gr-name"));
    let mut request_data = empty_request_data();
    let sink = DiscardSink;
    let mut record_context = context(&guardrail_base, &mut request_data, &sink);
    log_failure(&mut record_context, &call, "upstream timeout", &[])
        .expect("the failure record is written");
    let expected = json(
        r#"{"guardrail_name":"gr-name","guardrail_provider":"bedrock","guardrail_mode":"during_call","guardrail_response":{"error":"upstream timeout"},"guardrail_status":"guardrail_failed_to_respond","start_time":100.25,"end_time":102.75,"duration":2.5,"masked_entity_count":null}"#,
    );
    assert_eq!(record_json(&request_data), expected);
}
