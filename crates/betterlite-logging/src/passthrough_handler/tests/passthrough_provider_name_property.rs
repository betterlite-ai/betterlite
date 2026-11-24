use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;

use super::support::harness_named;

#[rstest]
#[case::openai(ProviderId::Openai)]
#[case::anthropic(ProviderId::Anthropic)]
#[case::cohere(ProviderId::Cohere)]
#[case::gemini(ProviderId::Gemini)]
#[case::vertex_ai(ProviderId::VertexAi)]
#[case::bedrock(ProviderId::Bedrock)]
fn every_provider_reports_its_own_name(#[case] name: ProviderId) {
    let harness = harness_named(name);
    assert_eq!(harness.handler.provider.llm_provider_name(), name);
}
