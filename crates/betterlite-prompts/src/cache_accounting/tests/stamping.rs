use std::collections::BTreeMap;

use rstest::rstest;

use crate::cache_accounting::dialect::DialectInputs;
use crate::cache_accounting::points::{
    CacheControlInjectionPoint, MessageInjectionPoint, ToolConfigInjectionPoint,
};
use crate::cache_accounting::stamping::{stamped_for_prompt_hook, stamped_with_dialect};
use crate::cache_accounting::tests::support::{Facts, dialect};

fn points() -> Vec<CacheControlInjectionPoint> {
    vec![
        CacheControlInjectionPoint::Message(MessageInjectionPoint {
            role: None,
            index: None,
            control: None,
            openai_dialect: None,
            external_breakpoints: None,
        }),
        CacheControlInjectionPoint::ToolConfig(ToolConfigInjectionPoint {
            control: None,
            openai_dialect: None,
            external_breakpoints: None,
        }),
    ]
}

fn openai_inputs<'input>(
    model: &'input str,
    provider: Option<&'input str>,
    environment: &'input BTreeMap<String, String>,
) -> DialectInputs<'input> {
    let mut inputs = dialect(model, environment);
    inputs.custom_llm_provider = provider;
    inputs.api_base = Some("https://api.openai.com/v1");
    inputs
}

fn stamps(stamped: &[CacheControlInjectionPoint]) -> Vec<(Option<bool>, Option<usize>)> {
    stamped
        .iter()
        .map(|injection_point| match injection_point {
            CacheControlInjectionPoint::Message(fields) => {
                (fields.openai_dialect, fields.external_breakpoints)
            }
            CacheControlInjectionPoint::ToolConfig(fields) => {
                (fields.openai_dialect, fields.external_breakpoints)
            }
        })
        .collect()
}

#[test]
fn stamped_with_dialect_leaves_points_untouched_without_model_support() {
    let environment = BTreeMap::new();
    let inputs = openai_inputs("claude-3-5-sonnet", Some("anthropic"), &environment);
    let facts = Facts::new(&[], None);
    assert_eq!(stamped_with_dialect(&points(), &inputs, &facts), points());
}

#[rstest]
#[case::openai_dialect(Some("openai"), true)]
#[case::other_provider(Some("anthropic"), false)]
fn stamped_with_dialect_records_the_decision_on_every_point(
    #[case] provider: Option<&str>,
    #[case] expected: bool,
) {
    let environment = BTreeMap::new();
    let inputs = openai_inputs("gpt-5.6", provider, &environment);
    let facts = Facts::new(&[], None);
    let stamped = stamped_with_dialect(&points(), &inputs, &facts);
    assert_eq!(
        stamps(&stamped),
        vec![(Some(expected), None), (Some(expected), None)]
    );
}

#[test]
fn stamped_for_prompt_hook_adds_the_external_breakpoint_count() {
    let environment = BTreeMap::new();
    let inputs = openai_inputs("gpt-5.6", Some("openai"), &environment);
    let facts = Facts::new(&[], None);
    let stamped = stamped_for_prompt_hook(&points(), 2, &inputs, &facts);
    assert_eq!(
        stamps(&stamped),
        vec![(Some(true), Some(2)), (Some(true), Some(2))]
    );
}

#[test]
fn stamped_for_prompt_hook_leaves_a_zero_count_unstamped() {
    let environment = BTreeMap::new();
    let inputs = openai_inputs("gpt-5.6", Some("openai"), &environment);
    let facts = Facts::new(&[], None);
    let stamped = stamped_for_prompt_hook(&points(), 0, &inputs, &facts);
    assert_eq!(
        stamps(&stamped),
        vec![(Some(true), None), (Some(true), None)]
    );
}

#[test]
fn stamped_for_prompt_hook_does_not_mutate_the_input_points() {
    let environment = BTreeMap::new();
    let inputs = openai_inputs("gpt-5.6", Some("openai"), &environment);
    let facts = Facts::new(&[], None);
    let original = points();
    let stamped = stamped_for_prompt_hook(&original, 3, &inputs, &facts);
    assert_ne!(stamped, original);
    assert_eq!(original, points());
}
