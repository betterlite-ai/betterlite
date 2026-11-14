#![cfg(test)]

use std::time::Duration;

use betterlite_config::sections::budget_windows as config;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::{BudgetPeriodError, BudgetWindowsError, PeriodParser, budget_windows};

fn section() -> config::Settings {
    let mut providers = IndexMap::new();
    providers.insert(
        "openai".to_owned(),
        config::BudgetValues {
            limit: Some(Decimal::ZERO),
            duration: Some("24h".to_owned()),
        },
    );
    let mut tags = IndexMap::new();
    tags.insert(
        "team".to_owned(),
        config::BudgetValues {
            limit: Some(Decimal::from(5_u8)),
            duration: Some("daily".to_owned()),
        },
    );
    config::Settings {
        provider_budget_config: Some(providers),
        tag_budget_config: Some(tags),
        deployments: vec![
            config::DeploymentBudgetConfig {
                max_budget: Some(Decimal::from(10_u8)),
                budget_duration: Some("1d".to_owned()),
            },
            config::DeploymentBudgetConfig {
                max_budget: None,
                budget_duration: None,
            },
        ],
    }
}

#[test]
fn converts_configured_budgets_into_typed_periods() {
    let now = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
    let database = TimeZoneDatabase::from_env();
    let parser = PeriodParser {
        now,
        timezone: Some("UTC"),
        database: &database,
    };
    let resolved = budget_windows(&section(), &[Some("d1"), Some("d2")], parser).unwrap();
    let provider = resolved
        .provider_budget_config
        .as_ref()
        .and_then(|configs| configs.get("openai"))
        .unwrap();
    assert_eq!(provider.budget_limit, Some(Decimal::ZERO));
    assert_eq!(
        provider.period.as_ref().map(|period| period.spec.as_str()),
        Some("24h")
    );
    assert_eq!(
        provider.period.as_ref().map(|period| period.seconds),
        Some(Duration::from_secs(86_400))
    );
    let tag = resolved
        .tag_budget_config
        .as_ref()
        .and_then(|configs| configs.get("team"))
        .unwrap();
    assert_eq!(tag.max_budget, Some(Decimal::from(5_u8)));
    assert_eq!(
        tag.period.as_ref().map(|period| period.seconds),
        Some(Duration::from_secs(86_400))
    );
}

#[test]
fn keeps_one_limit_entry_per_catalog_deployment() {
    let now = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
    let database = TimeZoneDatabase::from_env();
    let parser = PeriodParser {
        now,
        timezone: None,
        database: &database,
    };
    let resolved = budget_windows(&section(), &[Some("d1"), Some("d2")], parser).unwrap();
    assert_eq!(resolved.deployments.len(), 2);
    assert_eq!(
        resolved
            .deployments
            .first()
            .and_then(|limits| limits.max_budget),
        Some(Decimal::from(10_u8))
    );
    assert_eq!(
        resolved
            .deployments
            .first()
            .and_then(|limits| limits.model_id.as_deref()),
        Some("d1")
    );
    assert!(
        resolved
            .deployments
            .get(1)
            .is_some_and(|limits| limits.period.is_none())
    );
}

#[test]
fn a_missing_catalog_deployment_and_an_invalid_duration_are_named() {
    let now = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
    let database = TimeZoneDatabase::from_env();
    let parser = PeriodParser {
        now,
        timezone: None,
        database: &database,
    };
    let missing = budget_windows(&section(), &[Some("d1")], parser).unwrap_err();
    assert!(matches!(
        missing,
        BudgetWindowsError::MissingDeployment { index: 1 }
    ));
    let mut broken = section();
    broken.deployments = vec![config::DeploymentBudgetConfig {
        max_budget: Some(Decimal::ONE),
        budget_duration: Some("nope".to_owned()),
    }];
    let error = budget_windows(&broken, &[Some("d1")], parser).unwrap_err();
    assert!(matches!(
        error,
        BudgetWindowsError::Duration {
            source: BudgetPeriodError::Parse(_),
            ..
        }
    ));
}
