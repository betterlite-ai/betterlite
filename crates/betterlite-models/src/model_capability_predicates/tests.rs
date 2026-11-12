#![cfg(test)]

mod fixture;
mod scenarios;

use rstest::rstest;

use super::CapabilityRequest;
use super::details;

#[derive(Clone, Copy, Debug)]
enum Scenario {
    SupportsResolution,
    SupportsErrorFallback,
    DeclaredStringValue,
    ExplicitDisabled,
    NamedCapabilityWrappers,
    NativeStreamingDefault,
    ResponseSchemaProviderOverride,
}

// The declared pair is what resolution would return, but answering it here skips the OAuth flow.
#[test]
fn declared_target_reads_the_authenticating_provider() {
    let prefixed = CapabilityRequest {
        model: "github_copilot/gpt-4o",
        custom_llm_provider: None,
    };
    let from_prefix = details::declared_target(&prefixed).expect("a declared provider");
    assert_eq!(from_prefix.model, "gpt-4o");
    assert_eq!(from_prefix.provider, "github_copilot");
    let declared = CapabilityRequest {
        model: "gpt-4o",
        custom_llm_provider: Some("github_copilot"),
    };
    let from_argument = details::declared_target(&declared).expect("a declared provider");
    assert_eq!(from_argument.model, "gpt-4o");
    let plain = CapabilityRequest {
        model: "gpt-4o",
        custom_llm_provider: Some("openai"),
    };
    assert!(details::declared_target(&plain).is_none());
}

#[rstest]
#[case::supports_resolution(Scenario::SupportsResolution)]
#[case::supports_error_fallback(Scenario::SupportsErrorFallback)]
#[case::declared_string_value(Scenario::DeclaredStringValue)]
#[case::explicit_disabled(Scenario::ExplicitDisabled)]
#[case::named_capability_wrappers(Scenario::NamedCapabilityWrappers)]
#[case::native_streaming_default(Scenario::NativeStreamingDefault)]
#[case::response_schema_provider_override(Scenario::ResponseSchemaProviderOverride)]
#[tokio::test]
async fn model_capability_predicates_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::SupportsResolution => scenarios::supports_resolution().await,
        Scenario::SupportsErrorFallback => scenarios::supports_error_fallback().await,
        Scenario::DeclaredStringValue => scenarios::declared_string_value().await,
        Scenario::ExplicitDisabled => scenarios::explicit_disabled().await,
        Scenario::NamedCapabilityWrappers => scenarios::named_capability_wrappers().await,
        Scenario::NativeStreamingDefault => scenarios::native_streaming_default().await,
        Scenario::ResponseSchemaProviderOverride => {
            scenarios::response_schema_provider_override().await;
        }
    }
}
