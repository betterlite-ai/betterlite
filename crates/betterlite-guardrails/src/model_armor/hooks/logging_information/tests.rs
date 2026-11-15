#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{ApplyVerdict, append_post_call, record_apply_guardrail};
use crate::guardrail_metadata::GuardrailMetadataError;
use crate::model_armor::hooks::error_handling::HookError;
use crate::model_armor::hooks::tests::support::{
    CLEAN, NAME, Recorder, SANITIZED_TEXT, STANDARD, armor, armor_with, entries_of, field,
    logging_details, prompt_data, records, settings, text, value,
};
use crate::model_armor::hooks::timing::RecordTimes;

#[tokio::test]
async fn logging_information_records_the_post_call_fields() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    request.insert("start_time".to_owned(), GuardrailValue::Decimal(1.5));
    let mut details = logging_details();
    append_post_call(&hooks, &request, Some(&mut details), &value(CLEAN), false).expect("append");
    let entries = entries_of(&details);
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(field(entry, "guardrail_name"), text(NAME));
    assert_eq!(field(entry, "guardrail_provider"), text("model_armor"));
    assert_eq!(field(entry, "guardrail_mode"), text("post_call"));
    assert_eq!(field(entry, "guardrail_status"), text("success"));
    assert_eq!(field(entry, "start_time"), GuardrailValue::Decimal(1.5));
    assert_eq!(field(entry, "end_time"), GuardrailValue::Null);
    assert_eq!(field(entry, "duration"), GuardrailValue::Null);
    assert_eq!(field(entry, "masked_entity_count"), GuardrailValue::Null);
    let GuardrailValue::Object(fields) = entry else {
        panic!("expected a record object, got {entry:?}");
    };
    assert_eq!(fields.len(), 9);
}

#[tokio::test]
async fn logging_information_post_call_without_a_logging_object_writes_nothing() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let request = prompt_data();
    append_post_call(&hooks, &request, None, &value(CLEAN), false).expect("no details");
}

#[tokio::test]
async fn logging_information_post_call_records_a_null_start_time_when_absent() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let request = prompt_data();
    let mut details = logging_details();
    append_post_call(&hooks, &request, Some(&mut details), &value(CLEAN), false).expect("append");
    let entries = entries_of(&details);
    assert_eq!(field(&entries[0], "start_time"), GuardrailValue::Null);
}

#[tokio::test]
async fn logging_information_post_call_appends_to_the_existing_entries() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let request = prompt_data();
    let mut details = logging_details();
    details.insert(
        STANDARD.to_owned(),
        value(r#"{"guardrail_information":[{"guardrail_name":"first"}]}"#),
    );
    append_post_call(&hooks, &request, Some(&mut details), &value(CLEAN), false).expect("append");
    let entries = entries_of(&details);
    assert_eq!(entries.len(), 2);
    assert_eq!(field(&entries[0], "guardrail_name"), text("first"));
    assert_eq!(field(&entries[1], "guardrail_name"), text(NAME));
}

#[tokio::test]
async fn logging_information_post_call_rejects_a_logging_object_that_is_not_an_object() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let request = prompt_data();
    let mut details = BTreeMap::from([(STANDARD.to_owned(), text("x"))]);
    let error = append_post_call(&hooks, &request, Some(&mut details), &value(CLEAN), false)
        .expect_err("not typed");
    assert!(matches!(
        error,
        HookError::Metadata(GuardrailMetadataError::NotTyped {
            field: "standard_logging_object",
            ..
        })
    ));
}

#[tokio::test]
async fn logging_information_post_call_sanitizes_the_logged_response() {
    let fixture = armor_with(Vec::new(), settings(), true).await;
    let hooks = fixture.hooks();
    let request = prompt_data();
    let mut details = logging_details();
    append_post_call(
        &hooks,
        &request,
        Some(&mut details),
        &value(SANITIZED_TEXT),
        false,
    )
    .expect("append");
    let entries = entries_of(&details);
    assert_eq!(
        field(
            field(&entries[0], "guardrail_response"),
            "model_armor_response"
        ),
        value(r#"{"sanitizedText":"[REDACTED]"}"#)
    );
}

#[tokio::test]
async fn logging_information_apply_success_records_the_provider_and_the_times() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    let times = RecordTimes {
        start_time: 1.0,
        end_time: 2.5,
        duration: 1.5,
    };
    record_apply_guardrail(
        &hooks,
        &mut request,
        &mut recorder.context(),
        ApplyVerdict::Success {
            response: value(CLEAN),
        },
        times,
    )
    .expect("record");
    let entries = records(&request, "metadata");
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(field(entry, "guardrail_response"), value(CLEAN));
    assert_eq!(field(entry, "guardrail_status"), text("success"));
    assert_eq!(field(entry, "guardrail_provider"), text("model_armor"));
    assert_eq!(field(entry, "start_time"), GuardrailValue::Decimal(1.0));
    assert_eq!(field(entry, "end_time"), GuardrailValue::Decimal(2.5));
    assert_eq!(field(entry, "duration"), GuardrailValue::Decimal(1.5));
    assert_eq!(
        field(entry, "guardrail_mode"),
        value(r#"["pre_call","during_call","post_call"]"#)
    );
    assert!(recorder.invocation.self_recorded);
}

#[tokio::test]
async fn logging_information_apply_flagged_records_a_flagged_status() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    let times = RecordTimes {
        start_time: 1.0,
        end_time: 1.0,
        duration: 0.0,
    };
    record_apply_guardrail(
        &hooks,
        &mut request,
        &mut recorder.context(),
        ApplyVerdict::Flagged {
            response: value(CLEAN),
        },
        times,
    )
    .expect("record");
    let entries = records(&request, "metadata");
    assert_eq!(
        field(&entries[0], "guardrail_status"),
        text("guardrail_flagged")
    );
}

#[tokio::test]
async fn logging_information_apply_failure_records_the_detail() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(None);
    let times = RecordTimes {
        start_time: 1.0,
        end_time: 1.0,
        duration: 0.0,
    };
    record_apply_guardrail(
        &hooks,
        &mut request,
        &mut recorder.context(),
        ApplyVerdict::FailedToRespond {
            detail: "boom".to_owned(),
        },
        times,
    )
    .expect("record");
    let entries = records(&request, "metadata");
    assert_eq!(
        field(&entries[0], "guardrail_status"),
        text("guardrail_failed_to_respond")
    );
    assert_eq!(field(&entries[0], "guardrail_response"), text("boom"));
}
