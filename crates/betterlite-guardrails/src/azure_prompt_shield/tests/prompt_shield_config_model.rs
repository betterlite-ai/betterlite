use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::azure_content_safety::api_version::DEFAULT_API_VERSION;
use crate::azure_prompt_shield::config_model::AzurePromptShieldConfigModel;

#[test]
fn the_config_model_reads_the_billing_fields() {
    let model: AzurePromptShieldConfigModel = serde_json::from_str(
        r#"{"api_key": "sk-test", "api_base": "https://example.test",
            "api_version": "2023-01-01", "cost_tier": "paid",
            "price_per_1000_text_records": 0.5}"#,
    )
    .unwrap();
    assert_eq!(
        model
            .content_safety
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("sk-test".to_owned())
    );
    assert_eq!(
        model.content_safety.api_base.as_deref(),
        Some("https://example.test")
    );
    assert_eq!(model.content_safety.api_version, "2023-01-01");
    assert_eq!(model.cost_tier.as_deref(), Some("paid"));
    assert_eq!(
        model.price_per_1000_text_records,
        Some(Decimal::try_from(0.5).unwrap())
    );
}

#[test]
fn the_config_model_defaults_the_api_version() {
    let model: AzurePromptShieldConfigModel = serde_json::from_str("{}").unwrap();
    assert_eq!(model.content_safety.api_version, DEFAULT_API_VERSION);
    assert_eq!(model.cost_tier, None);
    assert_eq!(model.price_per_1000_text_records, None);
    assert_eq!(
        AzurePromptShieldConfigModel::default()
            .content_safety
            .api_version,
        DEFAULT_API_VERSION
    );
}

#[test]
fn a_text_price_reads_as_a_decimal() {
    let model: AzurePromptShieldConfigModel =
        serde_json::from_str(r#"{"price_per_1000_text_records": "0.25"}"#).unwrap();
    assert_eq!(model.price_per_1000_text_records, Some(Decimal::new(25, 2)));
}

#[test]
fn the_ui_friendly_name_is_the_litellm_one() {
    assert_eq!(
        AzurePromptShieldConfigModel::ui_friendly_name(),
        "Azure Content Safety Prompt Shield"
    );
}
