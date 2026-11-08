#![cfg(test)]

use rstest::rstest;

mod model_id_cases;
mod provider_cases;
mod region_cases;
mod runtime_cases;
mod sts_cases;

#[derive(Debug, Clone, Copy)]
enum Scenario {
    ModelArnRegion,
    ExplicitRegionPrecedence,
    EnvironmentProfileFallback,
    NonLlmRegionPrecedence,
    InvalidRegion,
    ArnModelPathEncoding,
    InferenceProfileModelId,
    ChinaPartition,
    RuntimeAgentAgentcoreEndpoints,
    StsRegionFromEndpoint,
    ProviderSelectionOrder,
    ModelIdEncodingAndMutation,
    LlmRegionPrecedence,
    StsRegionAndKwargs,
    RuntimeEndpointPrecedence,
}

#[rstest]
#[case::model_arn_region(Scenario::ModelArnRegion)]
#[case::explicit_region_precedence(Scenario::ExplicitRegionPrecedence)]
#[case::environment_profile_fallback(Scenario::EnvironmentProfileFallback)]
#[case::non_llm_region_precedence(Scenario::NonLlmRegionPrecedence)]
#[case::invalid_region(Scenario::InvalidRegion)]
#[case::arn_model_path_encoding(Scenario::ArnModelPathEncoding)]
#[case::inference_profile_model_id(Scenario::InferenceProfileModelId)]
#[case::china_partition(Scenario::ChinaPartition)]
#[case::runtime_agent_agentcore_endpoints(Scenario::RuntimeAgentAgentcoreEndpoints)]
#[case::sts_region_from_endpoint(Scenario::StsRegionFromEndpoint)]
#[case::provider_selection_order(Scenario::ProviderSelectionOrder)]
#[case::model_id_encoding_and_mutation(Scenario::ModelIdEncodingAndMutation)]
#[case::llm_region_precedence(Scenario::LlmRegionPrecedence)]
#[case::sts_region_and_kwargs(Scenario::StsRegionAndKwargs)]
#[case::runtime_endpoint_precedence(Scenario::RuntimeEndpointPrecedence)]
fn aws_endpoints_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::ModelArnRegion => region_cases::model_arn_region(),
        Scenario::ExplicitRegionPrecedence => region_cases::explicit_region_precedence(),
        Scenario::EnvironmentProfileFallback => region_cases::environment_profile_fallback(),
        Scenario::NonLlmRegionPrecedence => region_cases::non_llm_region_precedence(),
        Scenario::InvalidRegion => region_cases::invalid_region(),
        Scenario::ArnModelPathEncoding => model_id_cases::arn_model_path_encoding(),
        Scenario::InferenceProfileModelId => model_id_cases::inference_profile_model_id(),
        Scenario::ChinaPartition => runtime_cases::china_partition(),
        Scenario::RuntimeAgentAgentcoreEndpoints => {
            runtime_cases::runtime_agent_agentcore_endpoints();
        }
        Scenario::StsRegionFromEndpoint => sts_cases::sts_region_from_endpoint(),
        Scenario::ProviderSelectionOrder => provider_cases::provider_selection_order(),
        Scenario::ModelIdEncodingAndMutation => model_id_cases::model_id_encoding_and_mutation(),
        Scenario::LlmRegionPrecedence => region_cases::llm_region_precedence(),
        Scenario::StsRegionAndKwargs => sts_cases::sts_region_and_kwargs(),
        Scenario::RuntimeEndpointPrecedence => runtime_cases::runtime_endpoint_precedence(),
    }
}
