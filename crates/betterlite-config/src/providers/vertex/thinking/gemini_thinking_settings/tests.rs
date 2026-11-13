#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_env::EnvError;
use rstest::rstest;

use crate::types::LiteLlmSettings;

use super::GeminiThinkingSettings;

const DEFAULTS: [i64; 8] = [128, 1, 128, 512, 1024, 2048, 4096, 0];
const OVERRIDES: [i64; 8] = [101, 2, 103, 104, 105, 106, 107, 108];

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(
    configured_low: Option<bool>,
    environment: &[(&str, &str)],
) -> Result<GeminiThinkingSettings, EnvError> {
    LiteLlmSettings::gemini_thinking_settings(configured_low, &lookup(environment))
}

#[test]
fn loads_thinking_controls() {
    let (defaults, low_default) =
        load(None, &[]).expect("an unset environment takes every pinned default");
    assert_eq!(defaults, DEFAULTS);
    assert!(!low_default);

    let (overridden, low_overridden) = load(
        Some(true),
        &[
            ("DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET", "101"),
            (
                "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH",
                "2",
            ),
            (
                "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_PRO",
                "103",
            ),
            (
                "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH_LITE",
                "104",
            ),
            ("DEFAULT_REASONING_EFFORT_LOW_THINKING_BUDGET", "105"),
            ("DEFAULT_REASONING_EFFORT_MEDIUM_THINKING_BUDGET", "106"),
            ("DEFAULT_REASONING_EFFORT_HIGH_THINKING_BUDGET", "107"),
            ("DEFAULT_REASONING_EFFORT_DISABLE_THINKING_BUDGET", "108"),
        ],
    )
    .expect("every variable resolves");
    assert_eq!(overridden, OVERRIDES);
    assert!(low_overridden);

    let (_, low_off) = load(Some(false), &[]).expect("the module flag defaults to off");
    assert!(!low_off);
}

#[rstest]
#[case::minimal("DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET")]
#[case::minimal_flash("DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH")]
#[case::minimal_pro("DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_PRO")]
#[case::minimal_flash_lite(
    "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH_LITE"
)]
#[case::low("DEFAULT_REASONING_EFFORT_LOW_THINKING_BUDGET")]
#[case::medium("DEFAULT_REASONING_EFFORT_MEDIUM_THINKING_BUDGET")]
#[case::high("DEFAULT_REASONING_EFFORT_HIGH_THINKING_BUDGET")]
#[case::disable("DEFAULT_REASONING_EFFORT_DISABLE_THINKING_BUDGET")]
fn an_empty_variable_counts_as_unset(#[case] variable: &str) {
    let (budgets, _) = load(None, &[(variable, "")]).expect("an empty value counts as unset");

    assert_eq!(budgets, DEFAULTS);
}

#[rstest]
#[case::minimal("DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET")]
#[case::minimal_flash("DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH")]
#[case::minimal_pro("DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_PRO")]
#[case::minimal_flash_lite(
    "DEFAULT_REASONING_EFFORT_MINIMAL_THINKING_BUDGET_GEMINI_2_5_FLASH_LITE"
)]
#[case::low("DEFAULT_REASONING_EFFORT_LOW_THINKING_BUDGET")]
#[case::medium("DEFAULT_REASONING_EFFORT_MEDIUM_THINKING_BUDGET")]
#[case::high("DEFAULT_REASONING_EFFORT_HIGH_THINKING_BUDGET")]
#[case::disable("DEFAULT_REASONING_EFFORT_DISABLE_THINKING_BUDGET")]
fn a_non_numeric_variable_names_itself(#[case] variable: &str) {
    let error = load(None, &[(variable, "many")]).expect_err("a non-numeric value is refused");

    assert_eq!(
        error,
        EnvError::Number {
            name: variable.to_owned()
        }
    );
}
