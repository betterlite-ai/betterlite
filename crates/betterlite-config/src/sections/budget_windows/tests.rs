#![cfg(test)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::loader::ConfigLoader;
use crate::sections::budget_windows::{BudgetValues, DeploymentBudgetConfig, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.budget_windows)
        .map_err(|error| error.to_string())
}

#[test]
fn reads_provider_tag_and_deployment_budgets() {
    let source = "router_settings:\n  provider_budget_config:\n    openai:\n      budget_limit: 100\n      time_period: 1d\n    azure:\n      budget_limit: os.environ/AZURE_BUDGET\n      time_period: 7d\nlitellm_settings:\n  tag_budget_config:\n    team:\n      max_budget: 5.5\n      budget_duration: daily\nmodel_list:\n  - model_name: gpt\n    litellm_params:\n      model: openai/gpt-4o-mini\n      max_budget: 10\n      budget_duration: 24h\n  - model_name: plain\n    litellm_params:\n      model: openai/gpt-4o-mini\n";
    let resolved = settings(source, &[("AZURE_BUDGET", "25.75")]).unwrap();
    let providers = resolved.provider_budget_config.unwrap();
    assert_eq!(
        providers.get("openai"),
        Some(&BudgetValues {
            limit: Some(Decimal::from(100_u8)),
            duration: Some("1d".to_owned()),
        })
    );
    assert_eq!(
        providers.get("azure").map(|budget| budget.limit),
        Some(Some(Decimal::new(2575, 2)))
    );
    let tags = resolved.tag_budget_config.unwrap();
    assert_eq!(
        tags.get("team"),
        Some(&BudgetValues {
            limit: Some(Decimal::new(55, 1)),
            duration: Some("daily".to_owned()),
        })
    );
    assert_eq!(
        resolved.deployments,
        vec![
            DeploymentBudgetConfig {
                max_budget: Some(Decimal::from(10_u8)),
                budget_duration: Some("24h".to_owned()),
            },
            DeploymentBudgetConfig::default(),
        ]
    );
}

#[test]
fn empty_maps_stay_configured_and_empty_environment_is_unset() {
    let source = "router_settings:\n  provider_budget_config: {}\nlitellm_settings:\n  tag_budget_config:\n    team:\n      budget_duration: os.environ/EMPTY_DURATION\n";
    let resolved = settings(source, &[("EMPTY_DURATION", "")]).unwrap();
    assert_eq!(
        resolved.provider_budget_config.map(|configs| configs.len()),
        Some(0)
    );
    let tags = resolved.tag_budget_config.unwrap();
    assert_eq!(
        tags.get("team"),
        Some(&BudgetValues {
            limit: None,
            duration: None,
        })
    );
}

#[test]
fn tag_aliases_win_over_the_declared_fields() {
    let source = "litellm_settings:\n  tag_budget_config:\n    team:\n      max_budget: 5\n      budget_limit: 10\n      budget_duration: 1d\n      time_period: 7d\n    solo:\n      max_budget: 3\n      budget_duration: 2h\n";
    let tags = settings(source, &[]).unwrap().tag_budget_config.unwrap();
    assert_eq!(
        tags.get("team"),
        Some(&BudgetValues {
            limit: Some(Decimal::from(10_u8)),
            duration: Some("7d".to_owned()),
        })
    );
    assert_eq!(
        tags.get("solo"),
        Some(&BudgetValues {
            limit: Some(Decimal::from(3_u8)),
            duration: Some("2h".to_owned()),
        })
    );
}

#[test]
fn a_null_budget_entry_names_its_path() {
    let provider = "router_settings:\n  provider_budget_config:\n    openai: null\n";
    let provider_error = settings(provider, &[]).unwrap_err();
    assert!(provider_error.contains("router_settings.provider_budget_config.openai"));
    let tag = "litellm_settings:\n  tag_budget_config:\n    team: null\n";
    let tag_error = settings(tag, &[]).unwrap_err();
    assert!(tag_error.contains("litellm_settings.tag_budget_config.team"));
}

#[test]
fn a_non_numeric_budget_names_its_path() {
    let source = "model_list:\n  - model_name: gpt\n    litellm_params:\n      model: openai/gpt-4o-mini\n      max_budget: ten\n";
    let message = settings(source, &[]).unwrap_err();
    assert!(message.contains("model_list.0.litellm_params.max_budget"));
}
