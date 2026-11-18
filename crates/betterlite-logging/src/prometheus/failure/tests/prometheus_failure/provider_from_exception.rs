use betterlite_error::normalize::class::Class;

use crate::prometheus::failure::provider::extract_api_provider_from_exception;
use crate::prometheus::failure::tests::support::{label_exception, with_class, with_provider};

#[test]
fn a_rate_limit_error_names_its_provider() {
    let exception = with_provider(
        with_class(label_exception(), Class::RateLimitError, "RateLimitError"),
        "openai",
    );
    assert_eq!(
        extract_api_provider_from_exception(&exception).as_deref(),
        Some("openai")
    );
}

#[test]
fn another_exception_class_names_no_provider() {
    let exception = with_provider(
        with_class(
            label_exception(),
            Class::AuthenticationError,
            "AuthenticationError",
        ),
        "openai",
    );
    assert_eq!(extract_api_provider_from_exception(&exception), None);
}

#[test]
fn a_budget_error_names_no_provider() {
    let exception = with_provider(
        with_class(
            label_exception(),
            Class::BudgetExceededError,
            "BudgetExceededError",
        ),
        "openai",
    );
    assert_eq!(extract_api_provider_from_exception(&exception), None);
}

#[test]
fn a_rate_limit_error_without_a_provider_names_none() {
    let exception = with_class(label_exception(), Class::RateLimitError, "RateLimitError");
    assert_eq!(extract_api_provider_from_exception(&exception), None);
}

#[test]
fn a_rate_limit_error_with_an_empty_provider_names_none() {
    let exception = with_provider(
        with_class(label_exception(), Class::RateLimitError, "RateLimitError"),
        "",
    );
    assert_eq!(extract_api_provider_from_exception(&exception), None);
}

#[test]
fn the_proxy_placeholder_names_no_provider() {
    let exception = with_provider(
        with_class(label_exception(), Class::RateLimitError, "RateLimitError"),
        "litellm_proxy",
    );
    assert_eq!(extract_api_provider_from_exception(&exception), None);
}

#[test]
fn a_provider_that_starts_like_the_placeholder_is_kept() {
    let exception = with_provider(
        with_class(label_exception(), Class::RateLimitError, "RateLimitError"),
        "litellm_proxy_custom",
    );
    assert_eq!(
        extract_api_provider_from_exception(&exception).as_deref(),
        Some("litellm_proxy_custom")
    );
}
