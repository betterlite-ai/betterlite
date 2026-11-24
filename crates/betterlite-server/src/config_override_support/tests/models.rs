#![cfg(test)]

use betterlite_openai::openai_base_config::Object as JsonMap;
use serde_json::json;

use crate::config_override_support::models::{
    ConfigOverrideSettingsResponse, OverrideKind, field_schema,
};

#[test]
fn kinds_name_themselves_as_litellm_config_types() {
    assert_eq!(OverrideKind::HashicorpVault.as_str(), "hashicorp_vault");
    assert_eq!(OverrideKind::CyberArk.as_str(), "cyberark");
    assert_eq!(OverrideKind::CyberArk.to_string(), "cyberark");
}

#[test]
fn schema_reports_the_model_description_and_first_non_null_type() {
    let schema = field_schema(OverrideKind::CyberArk);
    assert_eq!(
        schema.pointer("/description"),
        Some(&json!(
            "Configuration for CyberArk Conjur secret manager integration."
        ))
    );
    assert_eq!(
        schema.pointer("/properties/cyberark_api_base/description"),
        Some(&json!(
            "The address of the CyberArk Conjur server (e.g., https://conjur.example.com)"
        ))
    );
    assert_eq!(
        schema.pointer("/properties/cyberark_api_base/type"),
        Some(&json!("string"))
    );
    assert_eq!(
        field_schema(OverrideKind::HashicorpVault).pointer("/properties/vault_token/type"),
        Some(&json!("string"))
    );
}

#[test]
fn the_settings_response_reports_the_three_fields() {
    let response = ConfigOverrideSettingsResponse {
        config_type: OverrideKind::CyberArk.as_str().to_owned(),
        values: JsonMap::new(),
        field_schema: field_schema(OverrideKind::CyberArk),
    };
    let encoded = serde_json::to_value(&response).unwrap();
    assert_eq!(encoded.pointer("/config_type"), Some(&json!("cyberark")));
    assert_eq!(encoded.pointer("/values"), Some(&json!({})));
    assert!(encoded.pointer("/field_schema/properties").is_some());
}
