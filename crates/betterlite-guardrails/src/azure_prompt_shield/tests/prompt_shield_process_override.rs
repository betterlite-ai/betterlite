use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{
    DiscardSink, Log, NOW, NoSecrets, build, cost, guardrail, priced, stored, text, unit,
    usage_with, value,
};
use crate::azure_prompt_shield::billing::BillingUsage;
use crate::azure_prompt_shield::error::PromptShieldError;
use crate::azure_prompt_shield::guardrail::AzurePromptShieldGuardrail;
use crate::azure_prompt_shield::process::{
    ErrorRecord, ResponseRecord, process_error, process_response,
};
use crate::guardrail_base::failure::GuardrailFailure;

#[test]
fn process_response_records_a_success_with_the_azure_provider() {
    let guardrail = guardrail("http://127.0.0.1:1");
    let mut log = Log::new(&guardrail.base, BTreeMap::new());
    process_response(
        &guardrail,
        &mut log.context(&DiscardSink),
        ResponseRecord {
            response: None,
            baseline: None,
            event_type: Some(GuardrailEventHooks::PreCall),
            timing: NOW,
            usage: BillingUsage::default(),
        },
    )
    .unwrap();
    let record = stored(&log.data).first().cloned().unwrap();
    assert_eq!(record.get("guardrail_status"), Some(&text("success")));
    assert_eq!(record.get("guardrail_provider"), Some(&text("azure")));
    assert_eq!(
        record.get("guardrail_name"),
        Some(&text("azure-prompt-shield"))
    );
    assert_eq!(record.get("guardrail_mode"), Some(&text("pre_call")));
    assert_eq!(record.get("guardrail_response"), Some(&value("{}")));
}

#[rstest]
#[case::unchanged(false, "allow")]
#[case::modified(true, "mask")]
fn process_response_summarizes_a_pre_call_prompt(#[case] modified: bool, #[case] expected: &str) {
    let guardrail = guardrail("http://127.0.0.1:1");
    let baseline = BTreeMap::from([(
        "messages".to_owned(),
        value(r#"[{"role":"user","content":"hi"}]"#),
    )]);
    let response = if modified {
        value(r#"{"messages":[{"role":"user","content":"replaced"}]}"#)
    } else {
        value(r#"{"messages":[{"role":"user","content":"hi"}]}"#)
    };
    let mut log = Log::new(&guardrail.base, BTreeMap::new());
    process_response(
        &guardrail,
        &mut log.context(&DiscardSink),
        ResponseRecord {
            response: Some(&response),
            baseline: Some(&baseline),
            event_type: Some(GuardrailEventHooks::PreCall),
            timing: NOW,
            usage: BillingUsage::default(),
        },
    )
    .unwrap();
    let record = stored(&log.data).first().cloned().unwrap();
    assert_eq!(record.get("guardrail_response"), Some(&text(expected)));
}

#[rstest]
#[case::intervention(
    GuardrailFailure::Http { status: 403, detail: "blocked".to_owned() },
    "guardrail_intervened",
    "blocked"
)]
#[case::server_error(
    GuardrailFailure::Http { status: 500, detail: "down".to_owned() },
    "guardrail_failed_to_respond",
    "down"
)]
#[case::other(
    GuardrailFailure::Other("weird".to_owned()),
    "guardrail_failed_to_respond",
    "weird"
)]
fn process_error_records_the_failure_status(
    #[case] failure: GuardrailFailure,
    #[case] status: &str,
    #[case] response: &str,
) {
    let guardrail = guardrail("http://127.0.0.1:1");
    let mut log = Log::new(&guardrail.base, BTreeMap::new());
    process_error(
        &guardrail,
        &mut log.context(&DiscardSink),
        &failure,
        ErrorRecord {
            event_type: Some(GuardrailEventHooks::PreCall),
            timing: NOW,
            usage: usage_with(&["bad prompt"]),
        },
    )
    .unwrap();
    let record = stored(&log.data).first().cloned().unwrap();
    assert_eq!(record.get("guardrail_status"), Some(&text(status)));
    assert_eq!(record.get("guardrail_provider"), Some(&text("azure")));
    assert_eq!(record.get("guardrail_response"), Some(&text(response)));
    assert_eq!(unit(&record, "requests"), 1);
    assert_eq!(unit(&record, "text_records"), 1);
}

#[test]
fn process_error_stores_deny_when_the_switch_is_on() {
    let guardrail = guardrail("http://127.0.0.1:1");
    let mut base = guardrail.base.clone();
    base.settings.switches.deny_text_on_error = true;
    let mut log = Log::new(&base, BTreeMap::new());
    process_error(
        &guardrail,
        &mut log.context(&DiscardSink),
        &GuardrailFailure::Http {
            status: 400,
            detail: "attack".to_owned(),
        },
        ErrorRecord {
            event_type: None,
            timing: NOW,
            usage: BillingUsage::default(),
        },
    )
    .unwrap();
    let record = stored(&log.data).first().cloned().unwrap();
    assert_eq!(record.get("guardrail_response"), Some(&text("deny")));
}

fn priced_guardrail() -> AzurePromptShieldGuardrail {
    build(
        priced(
            "http://127.0.0.1:1",
            Some(text("paid")),
            Some(GuardrailValue::Decimal(0.5)),
        ),
        &NoSecrets,
    )
}

fn response_record(prompt: &str) -> ResponseRecord<'static> {
    ResponseRecord {
        response: None,
        baseline: None,
        event_type: None,
        timing: NOW,
        usage: usage_with(&[prompt]),
    }
}

fn error_record(prompt: &str) -> ErrorRecord {
    ErrorRecord {
        event_type: None,
        timing: NOW,
        usage: usage_with(&[prompt]),
    }
}

#[test]
fn process_response_attaches_usage_and_keeps_the_cost_out_of_spend() {
    let guardrail = priced_guardrail();
    let prompt = "x".repeat(1000);
    let mut log = Log::new(&guardrail.base, BTreeMap::new());
    process_response(
        &guardrail,
        &mut log.context(&DiscardSink),
        response_record(&prompt),
    )
    .unwrap();
    let record = stored(&log.data).first().cloned().unwrap();
    assert_eq!(unit(&record, "text_records"), 1);
    assert_eq!(
        record.get("guardrail_cost_in_spend"),
        Some(&GuardrailValue::Boolean(false))
    );
    assert_eq!(cost(&record).map(f64::to_bits), Some(0.0005_f64.to_bits()));
    assert_eq!(
        record.get("start_time"),
        Some(&GuardrailValue::Decimal(1.0))
    );
}

#[test]
fn process_error_attaches_usage_and_keeps_the_cost_out_of_spend() {
    let guardrail = priced_guardrail();
    let prompt = "x".repeat(1000);
    let mut log = Log::new(&guardrail.base, BTreeMap::new());
    process_error(
        &guardrail,
        &mut log.context(&DiscardSink),
        &GuardrailFailure::Http {
            status: 400,
            detail: "blocked".to_owned(),
        },
        error_record(&prompt),
    )
    .unwrap();
    let record = stored(&log.data).first().cloned().unwrap();
    assert_eq!(unit(&record, "text_records"), 1);
    assert_eq!(
        record.get("guardrail_cost_in_spend"),
        Some(&GuardrailValue::Boolean(false))
    );
    assert_eq!(cost(&record).map(f64::to_bits), Some(0.0005_f64.to_bits()));
    assert_eq!(record.get("duration"), Some(&GuardrailValue::Decimal(0.5)));
}

#[test]
fn a_response_pricing_error_leaves_the_record_unwritten() {
    let mut guardrail = priced_guardrail();
    guardrail.price_per_1000_text_records = Some(Decimal::MAX);
    let prompt = "x".repeat(3000);
    let mut log = Log::new(&guardrail.base, BTreeMap::new());
    let error = process_response(
        &guardrail,
        &mut log.context(&DiscardSink),
        response_record(&prompt),
    )
    .unwrap_err();
    assert!(matches!(&error, PromptShieldError::Pricing(_)), "{error}");
    assert!(!log.data.contains_key("metadata"));
}

#[test]
fn an_error_pricing_error_leaves_the_record_unwritten() {
    let mut guardrail = priced_guardrail();
    guardrail.price_per_1000_text_records = Some(Decimal::MAX);
    let prompt = "x".repeat(3000);
    let mut log = Log::new(&guardrail.base, BTreeMap::new());
    let error = process_error(
        &guardrail,
        &mut log.context(&DiscardSink),
        &GuardrailFailure::Other("x".to_owned()),
        error_record(&prompt),
    )
    .unwrap_err();
    assert!(matches!(&error, PromptShieldError::Pricing(_)), "{error}");
    assert!(!log.data.contains_key("metadata"));
}
