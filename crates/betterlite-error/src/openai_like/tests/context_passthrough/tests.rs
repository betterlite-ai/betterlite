#![cfg(test)]

use rstest::rstest;

use super::*;
use crate::error::ErrorClass;

#[rstest]
#[case::model_and_provider_copied(Some("gpt-4".to_owned()), "my_provider")]
#[case::no_model(None, "my_provider")]
fn openai_like_errors_behavior_tests_behavior(
    #[case] model: Option<String>,
    #[case] provider: &str,
) {
    let error = facts(Some(401));
    let mapped = map_openai_like_exception(&error, "failed", provider, model.clone())
        .expect("a matching status always produces a class");
    let ErrorClass::Authentication(context) = mapped else {
        panic!("expected the authentication family");
    };
    assert_eq!(context.model, model);
    assert_eq!(context.provider, Some(provider.to_owned()));
    assert!(context.message.contains(provider));
    assert_eq!(context.max_retries, None);
    assert_eq!(context.num_retries, None);
}
