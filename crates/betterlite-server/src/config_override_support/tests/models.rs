#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

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
fn kinds_deserialize_from_litellm_config_type_names() {
    let vault: OverrideKind = serde_json::from_str(r#""hashicorp_vault""#).unwrap();
    let cyberark: OverrideKind = serde_json::from_str(r#""cyberark""#).unwrap();
    assert_eq!(vault, OverrideKind::HashicorpVault);
    assert_eq!(cyberark, OverrideKind::CyberArk);
    assert_eq!(
        serde_json::to_string(&OverrideKind::CyberArk).unwrap(),
        r#""cyberark""#
    );
    serde_json::from_str::<OverrideKind>(r#""cyber_ark""#).unwrap_err();
}

#[test]
fn schema_reports_the_model_description_and_first_non_null_type() {
    let schema = field_schema(OverrideKind::CyberArk);
    assert_eq!(
        schema.pointer("/description").and_then(JsonValue::as_str),
        Some("Configuration for CyberArk Conjur secret manager integration.")
    );
    assert_eq!(
        schema
            .pointer("/properties/cyberark_api_base/description")
            .and_then(JsonValue::as_str),
        Some("The address of the CyberArk Conjur server (e.g., https://conjur.example.com)")
    );
    assert_eq!(
        schema
            .pointer("/properties/cyberark_api_base/type")
            .and_then(JsonValue::as_str),
        Some("string")
    );
    assert_eq!(
        field_schema(OverrideKind::HashicorpVault)
            .pointer("/properties/vault_token/type")
            .and_then(JsonValue::as_str),
        Some("string")
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
    assert_eq!(
        encoded.pointer("/config_type").and_then(JsonValue::as_str),
        Some("cyberark")
    );
    assert_eq!(
        encoded.pointer("/values"),
        Some(&JsonValue::Object(JsonMap::new()))
    );
    assert!(encoded.pointer("/field_schema/properties").is_some());
}
