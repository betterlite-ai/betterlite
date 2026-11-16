#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;

use super::{field_str, guardrail_attrs, guardrail_provider, guardrail_type};
use crate::guardrails::usage_projections::records::{GuardrailRecord, UsageGuardrailParams};

fn info(pairs: &[(&str, GuardrailValue)]) -> OpaqueParams {
    OpaqueParams(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn record(
    id: Option<&str>,
    name: Option<&str>,
    provider: Option<&str>,
    guardrail_info: Option<OpaqueParams>,
) -> GuardrailRecord {
    GuardrailRecord {
        guardrail_id: id.map(str::to_owned),
        guardrail_name: name.map(str::to_owned),
        litellm_params: UsageGuardrailParams {
            guardrail: provider.map(str::to_owned),
        },
        guardrail_info,
    }
}

#[test]
fn field_access_reads_the_id_and_display_name() {
    let db_row = record(Some("uuid-1"), Some("friendly"), Some("lakera"), None);
    assert_eq!(
        guardrail_attrs(&db_row),
        ("uuid-1".to_owned(), "friendly".to_owned())
    );
}

#[test]
fn field_access_falls_back_to_the_id_for_the_name() {
    let unnamed = record(Some("uuid-1"), None, Some("lakera"), None);
    let empty = record(Some("uuid-1"), Some(""), Some("lakera"), None);
    assert_eq!(
        guardrail_attrs(&unnamed),
        ("uuid-1".to_owned(), "uuid-1".to_owned())
    );
    assert_eq!(
        guardrail_attrs(&empty),
        ("uuid-1".to_owned(), "uuid-1".to_owned())
    );
}

#[test]
fn field_access_gives_an_empty_name_without_an_id_or_name() {
    let bare = record(None, None, None, None);
    assert_eq!(guardrail_attrs(&bare), (String::new(), String::new()));
}

#[test]
fn field_access_reads_the_provider_from_litellm_params() {
    let presidio = record(Some("uuid-1"), Some("friendly"), Some("presidio"), None);
    assert_eq!(guardrail_provider(&presidio), "presidio");
    let absent = record(Some("uuid-1"), Some("friendly"), None, None);
    assert_eq!(guardrail_provider(&absent), "Unknown");
}

#[test]
fn field_access_reads_the_type_from_guardrail_info() {
    let typed = record(
        Some("uuid-1"),
        Some("friendly"),
        Some("lakera"),
        Some(info(&[("type", text("presidio")), ("id", text("pii-1"))])),
    );
    assert_eq!(guardrail_type(&typed), "presidio");
}

#[test]
fn field_access_defaults_the_type_when_info_is_absent_or_untyped() {
    let absent = record(Some("uuid-1"), Some("friendly"), Some("lakera"), None);
    let untyped = record(
        Some("uuid-1"),
        Some("friendly"),
        Some("lakera"),
        Some(info(&[("id", text("pii-1"))])),
    );
    let numeric = record(
        Some("uuid-1"),
        Some("friendly"),
        Some("lakera"),
        Some(info(&[("type", GuardrailValue::SignedInteger(7))])),
    );
    assert_eq!(guardrail_type(&absent), "Guardrail");
    assert_eq!(guardrail_type(&untyped), "Guardrail");
    assert_eq!(guardrail_type(&numeric), "7");
}

#[test]
fn field_access_returns_text_and_the_default_otherwise() {
    let mapping = info(&[
        ("type", text("presidio")),
        ("count", GuardrailValue::Boolean(true)),
    ]);
    assert_eq!(field_str(&mapping, "type", "Guardrail"), "presidio");
    assert_eq!(field_str(&mapping, "missing", "Guardrail"), "Guardrail");
    assert_eq!(field_str(&mapping, "count", "Guardrail"), "Guardrail");
    let nothing = info(&[("type", GuardrailValue::Null)]);
    assert_eq!(field_str(&nothing, "type", "Guardrail"), "Guardrail");
    let empty = BTreeMap::new();
    assert_eq!(
        field_str(&OpaqueParams(empty), "type", "Guardrail"),
        "Guardrail"
    );
}

#[test]
fn field_access_treats_absent_info_as_an_empty_mapping() {
    let absent = record(Some("uuid-1"), Some("friendly"), Some("lakera"), None);
    assert_eq!(guardrail_type(&absent), "Guardrail");
    let empty = record(
        Some("uuid-1"),
        Some("friendly"),
        Some("lakera"),
        Some(OpaqueParams(BTreeMap::new())),
    );
    assert_eq!(guardrail_type(&empty), "Guardrail");
    let present = record(
        Some("uuid-1"),
        Some("friendly"),
        Some("lakera"),
        Some(info(&[("type", text("presidio"))])),
    );
    assert_eq!(guardrail_type(&present), "presidio");
}

#[test]
fn field_access_stringifies_numbers_like_python_str() {
    let numbers = info(&[
        ("signed", GuardrailValue::SignedInteger(-7)),
        ("unsigned", GuardrailValue::UnsignedInteger(u64::MAX)),
        ("whole", GuardrailValue::Decimal(2.0)),
        ("fraction", GuardrailValue::Decimal(1.5)),
        ("large", GuardrailValue::Decimal(1.5e16)),
        ("small", GuardrailValue::Decimal(1.5e-7)),
        ("edge", GuardrailValue::Decimal(0.0001)),
    ]);
    assert_eq!(field_str(&numbers, "signed", "d"), "-7");
    assert_eq!(field_str(&numbers, "unsigned", "d"), "18446744073709551615");
    assert_eq!(field_str(&numbers, "whole", "d"), "2.0");
    assert_eq!(field_str(&numbers, "fraction", "d"), "1.5");
    assert_eq!(field_str(&numbers, "large", "d"), "1.5e+16");
    assert_eq!(field_str(&numbers, "small", "d"), "1.5e-07");
    assert_eq!(field_str(&numbers, "edge", "d"), "0.0001");
}
