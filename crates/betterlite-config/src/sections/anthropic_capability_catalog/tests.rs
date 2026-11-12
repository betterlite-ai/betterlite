#![cfg(test)]

mod bedrock;

use std::collections::BTreeMap;

use rstest::rstest;

use super::{
    HIGH_THINKING_BUDGET, LOW_THINKING_BUDGET, MAX_THINKING_BUDGET, MEDIUM_THINKING_BUDGET,
    MINIMAL_THINKING_BUDGET, Settings, XHIGH_THINKING_BUDGET,
};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .anthropic_capability_catalog)
}

#[derive(Clone, Copy)]
enum Variable {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    BedrockMin,
}

impl Variable {
    fn read(self, settings: &Settings) -> i64 {
        match self {
            Self::Minimal => settings.minimal,
            Self::Low => settings.low,
            Self::Medium => settings.medium,
            Self::High => settings.high,
            Self::Xhigh => settings.xhigh,
            Self::Max => settings.max,
            Self::BedrockMin => settings.bedrock_min,
        }
    }
}

#[derive(Clone, Copy)]
enum Case {
    Loads {
        name: &'static str,
        default: i64,
        variable: Variable,
    },
    UnrelatedKey,
}

fn assert_variable(name: &str, default: i64, variable: Variable) {
    assert_eq!(variable.read(&load("", &[]).unwrap()), default);
    assert_eq!(variable.read(&load("", &[(name, "77")]).unwrap()), 77);
    assert_eq!(variable.read(&load("", &[(name, "")]).unwrap()), default);
    assert_eq!(variable.read(&load("", &[(name, " 88 ")]).unwrap()), 88);
    assert_eq!(variable.read(&load("", &[(name, "-5")]).unwrap()), -5);
    assert!(matches!(
        load("", &[(name, "not-a-number")]),
        Err(ConfigError::InvalidInteger { path, .. }) if path == name
    ));
}

fn assert_unrelated_key_rejected() {
    let source = "litellm_settings:\n  anthropic_capability_catalog_unrelated_key: 1\n";
    assert!(matches!(
        load(source, &[]),
        Err(ConfigError::Unsupported { .. })
    ));
}

#[rstest]
#[case::loads_default_reasoning_effort_minimal_thinking_budget(Case::Loads {
    name: "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET",
    default: MINIMAL_THINKING_BUDGET,
    variable: Variable::Minimal,
})]
#[case::loads_default_reasoning_effort_low_thinking_budget(Case::Loads {
    name: "DEFAULT_REASONING_EFFORT_LOW_THINKING_BUDGET",
    default: LOW_THINKING_BUDGET,
    variable: Variable::Low,
})]
#[case::loads_default_reasoning_effort_medium_thinking_budget(Case::Loads {
    name: "DEFAULT_REASONING_EFFORT_MEDIUM_THINKING_BUDGET",
    default: MEDIUM_THINKING_BUDGET,
    variable: Variable::Medium,
})]
#[case::loads_default_reasoning_effort_high_thinking_budget(Case::Loads {
    name: "DEFAULT_REASONING_EFFORT_HIGH_THINKING_BUDGET",
    default: HIGH_THINKING_BUDGET,
    variable: Variable::High,
})]
#[case::loads_default_reasoning_effort_xhigh_thinking_budget(Case::Loads {
    name: "DEFAULT_REASONING_EFFORT_XHIGH_THINKING_BUDGET",
    default: XHIGH_THINKING_BUDGET,
    variable: Variable::Xhigh,
})]
#[case::loads_default_reasoning_effort_max_thinking_budget(Case::Loads {
    name: "DEFAULT_REASONING_EFFORT_MAX_THINKING_BUDGET",
    default: MAX_THINKING_BUDGET,
    variable: Variable::Max,
})]
#[case::rejects_unrelated_key(Case::UnrelatedKey)]
fn anthropic_thinking_config_cases(#[case] case: Case) {
    match case {
        Case::Loads {
            name,
            default,
            variable,
        } => assert_variable(name, default, variable),
        Case::UnrelatedKey => assert_unrelated_key_rejected(),
    }
}
