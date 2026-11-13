#![cfg(test)]

use rstest::rstest;

use super::{EnvironmentError, validate_environment};
use crate::deepeval::deepeval_logging::types::Environment;

#[rstest]
#[case::production("production", Environment::Production)]
#[case::development("development", Environment::Development)]
#[case::staging("staging", Environment::Staging)]
fn validate_environment_accepts_the_three_names(#[case] name: &str, #[case] expected: Environment) {
    assert_eq!(validate_environment(name).unwrap(), expected);
}

#[test]
fn validate_environment_rejects_another_name() {
    assert_eq!(
        validate_environment("qa").unwrap_err(),
        EnvironmentError::Invalid {
            environment: "qa".to_owned()
        }
    );
}
