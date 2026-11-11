#![cfg(test)]

mod fixture;
mod scenarios;

use rstest::rstest;

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
