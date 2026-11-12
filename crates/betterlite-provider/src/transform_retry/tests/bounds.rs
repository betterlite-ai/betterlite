use rstest::rstest;

use crate::transform_retry::{MAX_TRANSFORM_ATTEMPTS, TransformRetry, TransformRetryError};

#[rstest]
#[case::zero(0, false)]
#[case::one(1, true)]
#[case::the_largest(MAX_TRANSFORM_ATTEMPTS, true)]
#[case::past_the_largest(MAX_TRANSFORM_ATTEMPTS + 1, false)]
fn provider_framework_tests_behavior(#[case] attempts: u32, #[case] allowed: bool) {
    let result = TransformRetry::bounded(attempts);
    assert_eq!(result.is_ok(), allowed);
    if !allowed {
        assert_eq!(
            result.unwrap_err(),
            TransformRetryError::Attempts {
                max_attempts: attempts
            }
        );
    }
    assert_eq!(
        TransformRetry::DISABLED,
        TransformRetry::bounded(1).unwrap()
    );
}
