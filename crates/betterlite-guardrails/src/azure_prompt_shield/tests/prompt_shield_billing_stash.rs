use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::support::{
    CLEAN, DiscardSink, Log, NOW, NoSecrets, build, cost, guardrail, mock, priced, request, stored,
    text, unit, usage_with,
};
use crate::azure_prompt_shield::billing::BillingUsage;
use crate::azure_prompt_shield::guardrail::{AzurePromptShieldGuardrail, PreCallReturn};
use crate::azure_prompt_shield::process::{ResponseRecord, process_response};

fn empty_log(guardrail: &AzurePromptShieldGuardrail) -> Log {
    Log::new(&guardrail.base, BTreeMap::new())
}

#[tokio::test]
async fn the_counters_survive_a_request_data_swap_and_reach_the_record() {
    let server = mock(vec![CLEAN]).await;
    let guardrail = guardrail(&server.base);
    let data = request(r#"{"messages":[{"role":"user","content":"scan me"}]}"#);
    let mut usage = BillingUsage::default();
    let returned = guardrail
        .async_pre_call_hook(&data, "completion", &mut usage)
        .await
        .unwrap();
    assert_eq!(returned, PreCallReturn::None);
    let mut log = empty_log(&guardrail);
    process_response(
        &guardrail,
        &mut log.context(&DiscardSink),
        ResponseRecord {
            response: None,
            baseline: None,
            event_type: Some(GuardrailEventHooks::PreCall),
            timing: NOW,
            usage,
        },
    )
    .unwrap();
    let records = stored(&log.data);
    let record = records.first().unwrap();
    assert_eq!(unit(record, "requests"), 1);
    assert_eq!(unit(record, "input_characters"), 7);
    assert_eq!(unit(record, "text_records"), 1);
}

#[tokio::test]
async fn each_invocation_starts_the_counters_from_zero() {
    let server = mock(vec![CLEAN]).await;
    let guardrail = guardrail(&server.base);
    let data = request(r#"{"messages":[{"role":"user","content":"first prompt"}]}"#);
    let mut usage = BillingUsage::default();
    guardrail
        .async_pre_call_hook(&data, "completion", &mut usage)
        .await
        .unwrap();
    assert_eq!(usage, usage_with(&["first prompt"]));
    let without_messages = BTreeMap::from([("model".to_owned(), text("gpt-4"))]);
    let returned = guardrail
        .async_pre_call_hook(&without_messages, "completion", &mut usage)
        .await
        .unwrap();
    assert_eq!(returned, PreCallReturn::Data);
    assert_eq!(usage, BillingUsage::default());
}

#[test]
fn a_usage_free_invocation_records_no_tracing_detail() {
    let guardrail = guardrail("http://127.0.0.1:1");
    let mut log = empty_log(&guardrail);
    process_response(
        &guardrail,
        &mut log.context(&DiscardSink),
        ResponseRecord {
            response: None,
            baseline: None,
            event_type: None,
            timing: NOW,
            usage: BillingUsage::default(),
        },
    )
    .unwrap();
    let records = stored(&log.data);
    let record = records.first().unwrap();
    assert!(!record.contains_key("guardrail_usage"));
    assert!(!record.contains_key("guardrail_cost"));
    assert!(!record.contains_key("guardrail_cost_in_spend"));
}

#[test]
fn a_priced_usage_is_recorded_but_never_counted_in_spend() {
    let guardrail = build(
        priced(
            "http://127.0.0.1:1",
            Some(text("paid")),
            Some(GuardrailValue::Decimal(0.5)),
        ),
        &NoSecrets,
    );
    let long = "x".repeat(4800);
    let mut log = empty_log(&guardrail);
    process_response(
        &guardrail,
        &mut log.context(&DiscardSink),
        ResponseRecord {
            response: None,
            baseline: None,
            event_type: None,
            timing: NOW,
            usage: usage_with(&[long.as_str()]),
        },
    )
    .unwrap();
    let record = stored(&log.data).first().cloned().unwrap();
    assert_eq!(unit(&record, "input_characters"), 4800);
    assert_eq!(unit(&record, "text_records"), 5);
    assert_eq!(
        record.get("guardrail_cost_in_spend"),
        Some(&GuardrailValue::Boolean(false))
    );
    assert_eq!(cost(&record).map(f64::to_bits), Some(2.4_f64.to_bits()));
}
