#![cfg(test)]

use serde_json::Value;

use crate::guardrails::form_tables_a::tables_a;
use crate::guardrails::provider_forms::project::{project_model, with_friendly_name};

fn projected_table(provider: &str) -> Value {
    let tables = tables_a();
    let (_, model) = tables
        .iter()
        .find(|(key, _)| *key == provider)
        .expect("the tables hold every provider with a config model");
    let mut fields = project_model(model, 100).expect("the tables nest below the limit");
    with_friendly_name(model, &mut fields);
    Value::Object(fields)
}

fn expected(json: &str) -> Value {
    serde_json::from_str(json).expect("the expected form dictionary is JSON")
}

#[test]
fn table_decision_model_projects_like_litellm() {
    assert_eq!(
        projected_table("decision_model"),
        expected(include_str!("fixtures/decision-model.json"))
    );
}

#[test]
fn table_crowdstrike_aidr_projects_like_litellm() {
    assert_eq!(
        projected_table("crowdstrike_aidr"),
        expected(include_str!("fixtures/crowdstrike-aidr.json"))
    );
}

#[test]
fn table_panw_prisma_airs_projects_like_litellm() {
    assert_eq!(
        projected_table("panw_prisma_airs"),
        expected(include_str!("fixtures/panw-prisma-airs.json"))
    );
}

#[test]
fn table_noma_projects_like_litellm() {
    assert_eq!(
        projected_table("noma"),
        expected(include_str!("fixtures/noma.json"))
    );
}

#[test]
fn table_noma_v2_projects_like_litellm() {
    assert_eq!(
        projected_table("noma_v2"),
        expected(include_str!("fixtures/noma-v2.json"))
    );
}

#[test]
fn table_conduct_projects_like_litellm() {
    assert_eq!(
        projected_table("conduct"),
        expected(include_str!("fixtures/conduct.json"))
    );
}

#[test]
fn table_typesafe_projects_like_litellm() {
    assert_eq!(
        projected_table("typesafe"),
        expected(include_str!("fixtures/typesafe.json"))
    );
}

#[test]
fn table_dynamoai_projects_like_litellm() {
    assert_eq!(
        projected_table("dynamoai"),
        expected(include_str!("fixtures/dynamoai.json"))
    );
}

#[test]
fn table_headroom_projects_like_litellm() {
    assert_eq!(
        projected_table("headroom"),
        expected(include_str!("fixtures/headroom.json"))
    );
}

#[test]
fn table_promptguard_projects_like_litellm() {
    assert_eq!(
        projected_table("promptguard"),
        expected(include_str!("fixtures/promptguard.json"))
    );
}

#[test]
fn table_qualifire_projects_like_litellm() {
    assert_eq!(
        projected_table("qualifire"),
        expected(include_str!("fixtures/qualifire.json"))
    );
}

#[test]
fn table_block_code_execution_projects_like_litellm() {
    assert_eq!(
        projected_table("block_code_execution"),
        expected(include_str!("fixtures/block-code-execution.json"))
    );
}

#[test]
fn table_qostodian_nexus_projects_like_litellm() {
    assert_eq!(
        projected_table("qostodian_nexus"),
        expected(include_str!("fixtures/qostodian-nexus.json"))
    );
}

#[test]
fn table_hiddenlayer_projects_like_litellm() {
    assert_eq!(
        projected_table("hiddenlayer"),
        expected(include_str!("fixtures/hiddenlayer.json"))
    );
}

#[test]
fn table_javelin_projects_like_litellm() {
    assert_eq!(
        projected_table("javelin"),
        expected(include_str!("fixtures/javelin.json"))
    );
}

#[test]
fn table_mcp_end_user_permission_projects_like_litellm() {
    assert_eq!(
        projected_table("mcp_end_user_permission"),
        expected(include_str!("fixtures/mcp-end-user-permission.json"))
    );
}

#[test]
fn table_openai_moderation_projects_like_litellm() {
    assert_eq!(
        projected_table("openai_moderation"),
        expected(include_str!("fixtures/openai-moderation.json"))
    );
}

#[test]
fn table_xecguard_projects_like_litellm() {
    assert_eq!(
        projected_table("xecguard"),
        expected(include_str!("fixtures/xecguard.json"))
    );
}

#[test]
fn table_azure_prompt_shield_projects_like_litellm() {
    assert_eq!(
        projected_table("azure/prompt_shield"),
        expected(include_str!("fixtures/azure-prompt-shield.json"))
    );
}

#[test]
fn table_azure_text_moderations_projects_like_litellm() {
    assert_eq!(
        projected_table("azure/text_moderations"),
        expected(include_str!("fixtures/azure-text-moderations.json"))
    );
}

#[test]
fn table_alice_projects_like_litellm() {
    assert_eq!(
        projected_table("alice"),
        expected(include_str!("fixtures/alice.json"))
    );
}

#[test]
fn table_litellm_content_filter_projects_like_litellm() {
    assert_eq!(
        projected_table("litellm_content_filter"),
        expected(include_str!("fixtures/litellm-content-filter.json"))
    );
}

#[test]
fn table_zscaler_ai_guard_projects_like_litellm() {
    assert_eq!(
        projected_table("zscaler_ai_guard"),
        expected(include_str!("fixtures/zscaler-ai-guard.json"))
    );
}

#[test]
fn table_guardrails_ai_projects_like_litellm() {
    assert_eq!(
        projected_table("guardrails_ai"),
        expected(include_str!("fixtures/guardrails-ai.json"))
    );
}

#[test]
fn tables_a_keys_are_the_registry_names() {
    let keys: Vec<&str> = tables_a().iter().map(|(key, _)| *key).collect();
    assert_eq!(
        keys,
        [
            "decision_model",
            "crowdstrike_aidr",
            "panw_prisma_airs",
            "noma",
            "noma_v2",
            "conduct",
            "typesafe",
            "dynamoai",
            "headroom",
            "promptguard",
            "qualifire",
            "block_code_execution",
            "qostodian_nexus",
            "hiddenlayer",
            "javelin",
            "mcp_end_user_permission",
            "openai_moderation",
            "xecguard",
            "azure/prompt_shield",
            "azure/text_moderations",
            "alice",
            "litellm_content_filter",
            "zscaler_ai_guard",
            "guardrails_ai",
        ]
    );
}

#[test]
fn tables_a_omit_the_provider_without_a_config_model() {
    assert!(
        !tables_a()
            .iter()
            .any(|(key, _)| *key == "microsoft_purview")
    );
}
