use betterlite_error::normalize::class::Class;

use crate::logging_metadata::exception::ExceptionFacts;
use crate::prometheus::failure::exception::LabelException;

// The facts get_error_information reads off a rate-limit exception.
const FACTS: &str = r#"{
    "class_name": "RateLimitError",
    "message": "slow down",
    "status_code": 429,
    "code": "429",
    "llm_provider": "openai",
    "category": "vendor_rate_limit",
    "rate_limit_type": "requests"
}"#;

fn facts() -> ExceptionFacts {
    serde_json::from_str(FACTS).expect("the facts parse")
}

#[test]
fn from_facts_copies_every_label_fact() {
    let exception = LabelException::from_facts(&facts());
    assert_eq!(exception.class_name, "RateLimitError");
    assert!(matches!(exception.class, Class::RateLimitError));
    assert_eq!(exception.message.as_deref(), Some("slow down"));
    assert_eq!(exception.status_code.as_deref(), Some("429"));
    assert_eq!(exception.code.as_deref(), Some("429"));
    assert_eq!(exception.llm_provider.as_deref(), Some("openai"));
    assert_eq!(exception.category.as_deref(), Some("vendor_rate_limit"));
    assert_eq!(exception.rate_limit_type.as_deref(), Some("requests"));
    assert_eq!(exception.legacy_class_name, None);
}

#[test]
fn from_proxy_rate_limit_pins_the_legacy_class_name() {
    let exception = LabelException::from_proxy_rate_limit(&facts());
    assert_eq!(exception.class_name, "RateLimitError");
    assert!(matches!(exception.class, Class::RateLimitError));
    assert_eq!(
        exception.legacy_class_name.as_deref(),
        Some("HTTPException")
    );
    assert_eq!(exception.llm_provider.as_deref(), Some("openai"));
}
