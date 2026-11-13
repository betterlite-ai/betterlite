use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;

use crate::api::messages_native_prepare::fields::{
    native_filters_betas, native_supported_fields as supported_fields,
};

const NATIVE_FIELDS: [&str; 19] = [
    "messages",
    "model",
    "system",
    "max_tokens",
    "stop_sequences",
    "temperature",
    "top_p",
    "top_k",
    "tools",
    "tool_choice",
    "thinking",
    "context_management",
    "compaction",
    "output_format",
    "inference_geo",
    "speed",
    "output_config",
    "reasoning_effort",
    "safeguards",
];

const CROSS_PROVIDER_FIELDS: [&str; 18] = [
    "messages",
    "model",
    "system",
    "max_tokens",
    "stop_sequences",
    "temperature",
    "top_p",
    "top_k",
    "tools",
    "tool_choice",
    "thinking",
    "context_management",
    "output_format",
    "inference_geo",
    "speed",
    "output_config",
    "reasoning_effort",
    "safeguards",
];

#[rstest]
#[case::native(Some(ProviderId::Anthropic))]
#[case::null_provider(None)]
fn native_supported_fields(#[case] provider: Option<ProviderId>) {
    assert_eq!(supported_fields(provider), NATIVE_FIELDS.to_vec());
}

#[rstest]
#[case::bedrock(Some(ProviderId::Bedrock))]
#[case::vertex_ai(Some(ProviderId::VertexAi))]
#[case::azure_ai(Some(ProviderId::AzureAi))]
fn native_supported_fields_omits_only_compaction(#[case] provider: Option<ProviderId>) {
    assert_eq!(supported_fields(provider), CROSS_PROVIDER_FIELDS.to_vec());
}

#[rstest]
#[case::null_provider(None, false)]
#[case::anthropic(Some(ProviderId::Anthropic), false)]
#[case::bedrock(Some(ProviderId::Bedrock), true)]
#[case::vertex_ai(Some(ProviderId::VertexAi), true)]
fn native_beta_filtering_is_cross_provider_only(
    #[case] provider: Option<ProviderId>,
    #[case] filters: bool,
) {
    assert_eq!(native_filters_betas(provider), filters);
}
