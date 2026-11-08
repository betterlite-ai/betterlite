#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use super::special_headers::SpecialHeaders;
use super::supported_db_object_type::SupportedDbObjectType;

#[rstest]
#[case::authorization(SpecialHeaders::Authorization, "Authorization")]
#[case::api_key(SpecialHeaders::ApiKey, "API-Key")]
#[case::x_api_key(SpecialHeaders::XApiKey, "x-api-key")]
#[case::x_goog_api_key(SpecialHeaders::XGoogApiKey, "x-goog-api-key")]
#[case::ocp_apim_subscription_key(
    SpecialHeaders::OcpApimSubscriptionKey,
    "Ocp-Apim-Subscription-Key"
)]
#[case::x_litellm_api_key(SpecialHeaders::XLitellmApiKey, "x-litellm-api-key")]
#[case::x_mcp_auth(SpecialHeaders::McpAuth, "x-mcp-auth")]
#[case::x_mcp_servers(SpecialHeaders::McpServers, "x-mcp-servers")]
#[case::x_mcp_access_groups(SpecialHeaders::McpAccessGroups, "x-mcp-access-groups")]
fn every_special_header_renders_its_value(#[case] header: SpecialHeaders, #[case] text: &str) {
    assert_eq!(header.as_str(), text);
}

#[test]
fn credential_header_names_are_the_six_lowercased_auth_headers() {
    let expected = BTreeSet::from([
        "api-key".to_owned(),
        "authorization".to_owned(),
        "ocp-apim-subscription-key".to_owned(),
        "x-api-key".to_owned(),
        "x-goog-api-key".to_owned(),
        "x-litellm-api-key".to_owned(),
    ]);
    assert_eq!(SpecialHeaders::litellm_credential_header_names(), expected);
}

#[rstest]
#[case::models(SupportedDbObjectType::Models, "models")]
#[case::mcp(SupportedDbObjectType::Mcp, "mcp")]
#[case::guardrails(SupportedDbObjectType::Guardrails, "guardrails")]
#[case::policies(SupportedDbObjectType::Policies, "policies")]
#[case::vector_stores(SupportedDbObjectType::VectorStores, "vector_stores")]
#[case::pass_through_endpoints(
    SupportedDbObjectType::PassThroughEndpoints,
    "pass_through_endpoints"
)]
#[case::prompts(SupportedDbObjectType::Prompts, "prompts")]
#[case::model_cost_map(SupportedDbObjectType::ModelCostMap, "model_cost_map")]
#[case::tools(SupportedDbObjectType::Tools, "tools")]
#[case::config_overrides(SupportedDbObjectType::ConfigOverrides, "config_overrides")]
#[case::websearch_interception_settings(
    SupportedDbObjectType::WebsearchInterceptionSettings,
    "websearch_interception_settings"
)]
fn every_db_object_type_renders_its_value(
    #[case] object_type: SupportedDbObjectType,
    #[case] text: &str,
) {
    assert_eq!(object_type.to_string(), text);
    assert_eq!(
        serde_json::to_value(object_type).unwrap(),
        serde_json::json!(text)
    );
    assert_eq!(
        serde_json::from_value::<SupportedDbObjectType>(serde_json::json!(text)).unwrap(),
        object_type
    );
}
