#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;

use rstest::rstest;

use super::bedrock_checks::{
    BedrockChecksError, BedrockContentFilterCategory, BedrockPromptAttackCategory,
    BedrockSensitiveInfoEntity,
};
use super::bedrock_streaming::BedrockGuardrailStreamParams;
use super::{
    BedrockChecksConfigModel, BedrockGuardrailConfigModel, DeepKeepGuardrailConfigModel,
    JavelinGuardrailConfigModel, LakeraOnFlagged, LakeraV2GuardrailConfigModel,
    LassoGuardrailConfigModel, NomaGuardrailConfigModel, PillarGuardrailConfigModel,
    ZscalerAIGuardConfigModel,
};
use crate::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;
use serde::de::DeserializeOwned;

fn extras(entries: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn assert_literal_round_trip<T>(wire: &str, value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let parsed: T = serde_json::from_str(&format!("\"{wire}\"")).unwrap();
    assert_eq!(&parsed, value);
    assert_eq!(serde_json::to_string(value).unwrap(), format!("\"{wire}\""));
}

#[test]
fn bedrock_checks_validator_rejects_no_check() {
    let error = serde_json::from_str::<BedrockChecksConfigModel>("{}").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Bedrock 'checks' must enable at least one of: contentFilter, promptAttack, sensitiveInformation.")
    );
}

#[rstest]
#[case::content_filter(r#"{"contentFilter":{"categories":[{"category":"VIOLENCE"}]}}"#)]
#[case::prompt_attack(r#"{"promptAttack":{"categories":[{"category":"JAILBREAK"}]}}"#)]
#[case::sensitive_information(r#"{"sensitiveInformation":{"entities":[{"type":"EMAIL"}]}}"#)]
fn bedrock_checks_validator_accepts_a_single_check(#[case] body: &str) {
    serde_json::from_str::<BedrockChecksConfigModel>(body).unwrap();
}

#[test]
fn bedrock_checks_validator_error_text_matches_litellm() {
    let model = BedrockChecksConfigModel {
        content_filter: None,
        prompt_attack: None,
        sensitive_information: None,
    };
    let error = model.require_at_least_one_check().unwrap_err();
    assert_eq!(
        error.to_string(),
        "Bedrock 'checks' must enable at least one of: contentFilter, promptAttack, sensitiveInformation."
    );
    assert!(matches!(error, BedrockChecksError::NoCheckEnabled));
}

#[test]
fn bedrock_guardrail_defaults() {
    let model: BedrockGuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert_eq!(model.disable_exception_on_block, Some(false));
    assert_eq!(model.chunk_budget_chars.0, 25_000);
    assert_eq!(
        model.content_filter_threshold.map(|threshold| threshold.0),
        Some(0.5_f64)
    );
    assert_eq!(
        model.prompt_attack_threshold.map(|threshold| threshold.0),
        Some(0.5_f64)
    );
    assert_eq!(
        model.pii_confidence_threshold.map(|threshold| threshold.0),
        Some(0.5_f64)
    );
    assert!(!model.contextual_grounding);
    assert!(model.checks.is_none());
    assert!(model.guardrail_identifier.is_none());
    assert!(model.aws_secret_access_key.is_none());
    assert!(model.aws_session_token.is_none());
}

#[test]
fn bedrock_stream_params_defaults() {
    let params: BedrockGuardrailStreamParams = serde_json::from_str("{}").unwrap();
    assert!(params.buffer_until_moderated);
    assert_eq!(params.streaming_sampling_rate.0, 5);
    assert!(!params.streaming_end_of_stream_only);
    assert!(!params.buffer_release_on_scan);
    assert_eq!(params, BedrockGuardrailStreamParams::default());
}

#[test]
fn lakera_v2_defaults() {
    let model: LakeraV2GuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert_eq!(model.payload, Some(true));
    assert_eq!(model.breakdown, Some(true));
    assert_eq!(model.dev_info, Some(true));
    assert_eq!(model.on_flagged, Some(LakeraOnFlagged::Block));
    assert!(model.api_key.is_none());
    assert!(model.api_base.is_none());
    assert!(model.project_id.is_none());
    assert!(model.metadata.is_none());
    assert!(model.advisory_system_message.is_none());
}

#[test]
fn lasso_defaults() {
    let model: LassoGuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert!(model.lasso_user_id.is_none());
    assert!(model.lasso_conversation_id.is_none());
    assert_eq!(model.mask, Some(false));
}

#[test]
fn deepkeep_defaults() {
    let model: DeepKeepGuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert!(model.deepkeep_firewall_id.is_none());
}

#[test]
fn pillar_defaults() {
    let model: PillarGuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert_eq!(model.on_flagged_action.as_deref(), Some("monitor"));
    assert_eq!(model.include_scanners, Some(true));
    assert_eq!(model.include_evidence, Some(true));
    assert!(model.async_mode.is_none());
    assert!(model.persist_session.is_none());
}

#[test]
fn noma_defaults() {
    let model: NomaGuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert_eq!(model.use_v2, Some(false));
    assert!(model.application_id.is_none());
    assert!(model.gateway_name.is_none());
    assert!(model.monitor_mode.is_none());
    assert!(model.block_failures.is_none());
    assert!(model.anonymize_input.is_none());
}

#[test]
fn zscaler_defaults() {
    let model: ZscalerAIGuardConfigModel = serde_json::from_str("{}").unwrap();
    assert!(model.policy_id.is_none());
    assert_eq!(model.send_user_api_key_alias, Some(false));
    assert_eq!(model.send_user_api_key_user_id, Some(false));
    assert_eq!(model.send_user_api_key_team_id, Some(false));
}

#[test]
fn javelin_defaults() {
    let model: JavelinGuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert!(model.guard_name.is_none());
    assert!(model.api_version.is_none());
    assert!(model.metadata.is_none());
    assert!(model.application.is_none());
    assert!(model.config.is_none());
}

#[test]
fn from_extras_keeps_known_fields_and_ignores_unknown() {
    let extras = extras(&[
        (
            "streaming_buffer_until_moderated",
            GuardrailValue::Boolean(false),
        ),
        (
            "streaming_sampling_rate",
            GuardrailValue::UnsignedInteger(2),
        ),
        ("unknown_key", GuardrailValue::Boolean(true)),
    ]);
    let params = BedrockGuardrailStreamParams::from_extras(Some(&extras)).unwrap();
    assert!(!params.buffer_until_moderated);
    assert_eq!(params.streaming_sampling_rate.0, 2);
    assert!(!params.streaming_end_of_stream_only);
}

#[test]
fn from_extras_skips_null_values() {
    let extras = extras(&[("streaming_sampling_rate", GuardrailValue::Null)]);
    let params = BedrockGuardrailStreamParams::from_extras(Some(&extras)).unwrap();
    assert_eq!(params.streaming_sampling_rate.0, 5);
}

#[test]
fn from_extras_without_extras_uses_defaults() {
    let params = BedrockGuardrailStreamParams::from_extras(None).unwrap();
    assert_eq!(params, BedrockGuardrailStreamParams::default());
}

#[test]
fn from_extras_empty_extras_uses_defaults() {
    let params = BedrockGuardrailStreamParams::from_extras(Some(&BTreeMap::new())).unwrap();
    assert_eq!(params, BedrockGuardrailStreamParams::default());
}

#[test]
fn from_extras_rejects_a_wrong_type() {
    let extras = extras(&[(
        "streaming_sampling_rate",
        GuardrailValue::Text("many".to_owned()),
    )]);
    BedrockGuardrailStreamParams::from_extras(Some(&extras)).unwrap_err();
}

#[rstest]
#[case::violence("VIOLENCE", BedrockContentFilterCategory::Violence)]
#[case::hate("HATE", BedrockContentFilterCategory::Hate)]
#[case::sexual("SEXUAL", BedrockContentFilterCategory::Sexual)]
#[case::misconduct("MISCONDUCT", BedrockContentFilterCategory::Misconduct)]
#[case::insults("INSULTS", BedrockContentFilterCategory::Insults)]
fn bedrock_content_filter_literals(
    #[case] wire: &str,
    #[case] value: BedrockContentFilterCategory,
) {
    assert_literal_round_trip(wire, &value);
}

#[rstest]
#[case::jailbreak("JAILBREAK", BedrockPromptAttackCategory::Jailbreak)]
#[case::prompt_injection("PROMPT_INJECTION", BedrockPromptAttackCategory::PromptInjection)]
#[case::prompt_leakage("PROMPT_LEAKAGE", BedrockPromptAttackCategory::PromptLeakage)]
fn bedrock_prompt_attack_literals(#[case] wire: &str, #[case] value: BedrockPromptAttackCategory) {
    assert_literal_round_trip(wire, &value);
}

#[rstest]
#[case::address("ADDRESS", BedrockSensitiveInfoEntity::Address)]
#[case::age("AGE", BedrockSensitiveInfoEntity::Age)]
#[case::aws_access_key("AWS_ACCESS_KEY", BedrockSensitiveInfoEntity::AwsAccessKey)]
#[case::aws_secret_key("AWS_SECRET_KEY", BedrockSensitiveInfoEntity::AwsSecretKey)]
#[case::ca_health_number("CA_HEALTH_NUMBER", BedrockSensitiveInfoEntity::CaHealthNumber)]
#[case::ca_social_insurance_number(
    "CA_SOCIAL_INSURANCE_NUMBER",
    BedrockSensitiveInfoEntity::CaSocialInsuranceNumber
)]
#[case::credit_debit_card_cvv(
    "CREDIT_DEBIT_CARD_CVV",
    BedrockSensitiveInfoEntity::CreditDebitCardCvv
)]
#[case::credit_debit_card_expiry(
    "CREDIT_DEBIT_CARD_EXPIRY",
    BedrockSensitiveInfoEntity::CreditDebitCardExpiry
)]
#[case::credit_debit_card_number(
    "CREDIT_DEBIT_CARD_NUMBER",
    BedrockSensitiveInfoEntity::CreditDebitCardNumber
)]
#[case::driver_id("DRIVER_ID", BedrockSensitiveInfoEntity::DriverId)]
#[case::email("EMAIL", BedrockSensitiveInfoEntity::Email)]
#[case::international_bank_account_number(
    "INTERNATIONAL_BANK_ACCOUNT_NUMBER",
    BedrockSensitiveInfoEntity::InternationalBankAccountNumber
)]
#[case::ip_address("IP_ADDRESS", BedrockSensitiveInfoEntity::IpAddress)]
#[case::license_plate("LICENSE_PLATE", BedrockSensitiveInfoEntity::LicensePlate)]
#[case::mac_address("MAC_ADDRESS", BedrockSensitiveInfoEntity::MacAddress)]
#[case::name("NAME", BedrockSensitiveInfoEntity::Name)]
#[case::password("PASSWORD", BedrockSensitiveInfoEntity::Password)]
#[case::phone("PHONE", BedrockSensitiveInfoEntity::Phone)]
#[case::pin("PIN", BedrockSensitiveInfoEntity::Pin)]
#[case::swift_code("SWIFT_CODE", BedrockSensitiveInfoEntity::SwiftCode)]
#[case::uk_national_health_service_number(
    "UK_NATIONAL_HEALTH_SERVICE_NUMBER",
    BedrockSensitiveInfoEntity::UkNationalHealthServiceNumber
)]
#[case::uk_national_insurance_number(
    "UK_NATIONAL_INSURANCE_NUMBER",
    BedrockSensitiveInfoEntity::UkNationalInsuranceNumber
)]
#[case::uk_unique_taxpayer_reference_number(
    "UK_UNIQUE_TAXPAYER_REFERENCE_NUMBER",
    BedrockSensitiveInfoEntity::UkUniqueTaxpayerReference
)]
#[case::url("URL", BedrockSensitiveInfoEntity::Url)]
#[case::username("USERNAME", BedrockSensitiveInfoEntity::Username)]
#[case::us_bank_account_number(
    "US_BANK_ACCOUNT_NUMBER",
    BedrockSensitiveInfoEntity::UsBankAccountNumber
)]
#[case::us_bank_routing_number(
    "US_BANK_ROUTING_NUMBER",
    BedrockSensitiveInfoEntity::UsBankRoutingNumber
)]
#[case::us_individual_tax_identification_number(
    "US_INDIVIDUAL_TAX_IDENTIFICATION_NUMBER",
    BedrockSensitiveInfoEntity::UsIndividualTaxId
)]
#[case::us_passport_number("US_PASSPORT_NUMBER", BedrockSensitiveInfoEntity::UsPassportNumber)]
#[case::us_social_security_number(
    "US_SOCIAL_SECURITY_NUMBER",
    BedrockSensitiveInfoEntity::UsSocialSecurityNumber
)]
#[case::vehicle_identification_number(
    "VEHICLE_IDENTIFICATION_NUMBER",
    BedrockSensitiveInfoEntity::VehicleIdentificationNumber
)]
fn bedrock_sensitive_info_literals(#[case] wire: &str, #[case] value: BedrockSensitiveInfoEntity) {
    assert_literal_round_trip(wire, &value);
}

#[test]
fn bedrock_unknown_category_is_rejected() {
    let body = r#"{"contentFilter":{"categories":[{"category":"BANANA"}]}}"#;
    serde_json::from_str::<BedrockChecksConfigModel>(body).unwrap_err();
}

#[rstest]
#[case::zero(0.0_f64, true)]
#[case::half(0.5_f64, true)]
#[case::one(1.0_f64, true)]
#[case::below(-0.1_f64, false)]
#[case::above(1.1_f64, false)]
fn thresholds_must_be_a_unit_score(#[case] value: f64, #[case] accepted: bool) {
    let body = format!(r#"{{"content_filter_threshold":{value}}}"#);
    assert_eq!(
        serde_json::from_str::<BedrockGuardrailConfigModel>(&body).is_ok(),
        accepted
    );
}

#[test]
fn chunk_budget_must_be_positive() {
    let model: BedrockGuardrailConfigModel =
        serde_json::from_str(r#"{"chunk_budget_chars":1}"#).unwrap();
    assert_eq!(model.chunk_budget_chars.0, 1);
    serde_json::from_str::<BedrockGuardrailConfigModel>(r#"{"chunk_budget_chars":0}"#).unwrap_err();
}

#[test]
fn sampling_rate_must_be_positive() {
    let params: BedrockGuardrailStreamParams =
        serde_json::from_str(r#"{"streaming_sampling_rate":1}"#).unwrap();
    assert_eq!(params.streaming_sampling_rate.0, 1);
    serde_json::from_str::<BedrockGuardrailStreamParams>(r#"{"streaming_sampling_rate":0}"#)
        .unwrap_err();
}

#[test]
fn lakera_v2_literal_on_flagged_values_are_the_three_litellm_actions() {
    let monitor: LakeraV2GuardrailConfigModel =
        serde_json::from_str(r#"{"on_flagged":"monitor"}"#).unwrap();
    assert_eq!(monitor.on_flagged, Some(LakeraOnFlagged::Monitor));
    let inject: LakeraV2GuardrailConfigModel =
        serde_json::from_str(r#"{"on_flagged":"inject_system_message"}"#).unwrap();
    assert_eq!(
        inject.on_flagged,
        Some(LakeraOnFlagged::InjectSystemMessage)
    );
    serde_json::from_str::<LakeraV2GuardrailConfigModel>(r#"{"on_flagged":"drop"}"#).unwrap_err();
}

#[test]
fn bedrock_secret_fields_serialize_in_clear() {
    let body = r#"{"aws_secret_access_key":"secret","aws_session_token":"token","aws_web_identity_token":"web"}"#;
    let model: BedrockGuardrailConfigModel = serde_json::from_str(body).unwrap();
    let encoded = serde_json::to_value(&model).unwrap();
    assert_eq!(encoded["aws_secret_access_key"], "secret");
    assert_eq!(encoded["aws_session_token"], "token");
    assert_eq!(encoded["aws_web_identity_token"], "web");
}
