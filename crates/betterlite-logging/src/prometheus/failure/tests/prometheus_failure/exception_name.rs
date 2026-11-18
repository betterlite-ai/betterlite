use betterlite_error::normalize::class::Class;

use crate::prometheus::failure::exception::LabelException;
use crate::prometheus::failure::exception_name::exception_class_name;
use crate::prometheus::failure::tests::support::{with_class, with_legacy, with_provider};

#[test]
fn the_legacy_marker_is_honored() {
    let exception = with_legacy(
        with_provider(
            with_class(
                LabelException::default(),
                Class::RateLimitError,
                "RateLimitError",
            ),
            "openai",
        ),
        "HTTPException",
    );
    assert_eq!(exception_class_name(&exception), "HTTPException");
}

#[test]
fn a_budget_error_stays_unprefixed() {
    let exception = with_provider(
        with_class(
            LabelException::default(),
            Class::BudgetExceededError,
            "BudgetExceededError",
        ),
        "openai",
    );
    assert_eq!(exception_class_name(&exception), "BudgetExceededError");
}

#[test]
fn the_provider_prefixes_the_class_name() {
    let exception = with_provider(
        with_class(
            LabelException::default(),
            Class::RateLimitError,
            "RateLimitError",
        ),
        "openai",
    );
    assert_eq!(exception_class_name(&exception), "Openai.RateLimitError");
}

#[test]
fn only_the_first_provider_character_is_uppercased() {
    let exception = with_provider(
        with_class(
            LabelException::default(),
            Class::AuthenticationError,
            "AuthenticationError",
        ),
        "azureOpenai",
    );
    assert_eq!(
        exception_class_name(&exception),
        "AzureOpenai.AuthenticationError"
    );
}

#[test]
fn a_single_character_provider_keeps_the_dot() {
    let exception = with_provider(
        with_class(LabelException::default(), Class::Other, "APIError"),
        "x",
    );
    assert_eq!(exception_class_name(&exception), "X.APIError");
}

#[test]
fn a_missing_provider_leaves_the_class_name_alone() {
    let exception = with_class(LabelException::default(), Class::Other, "APIError");
    assert_eq!(exception_class_name(&exception), "APIError");
}

#[test]
fn an_empty_provider_leaves_the_class_name_alone() {
    let exception = with_provider(
        with_class(LabelException::default(), Class::Other, "APIError"),
        "",
    );
    assert_eq!(exception_class_name(&exception), "APIError");
}

#[test]
fn an_empty_legacy_marker_is_ignored() {
    let exception = with_legacy(
        with_provider(
            with_class(
                LabelException::default(),
                Class::RateLimitError,
                "RateLimitError",
            ),
            "openai",
        ),
        "",
    );
    assert_eq!(exception_class_name(&exception), "Openai.RateLimitError");
}
