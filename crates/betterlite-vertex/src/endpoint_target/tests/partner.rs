#![cfg(test)]

use rstest::rstest;

use crate::endpoint_target::error::EndpointTargetError;
use crate::endpoint_target::partner::{Partner, create_vertex_url};
use crate::endpoint_target::target::get_complete_vertex_url;
use crate::model_urls::error::VertexUrlError;

use super::support::catalog;

#[derive(Debug, Clone, Copy)]
enum PartnerUrlKind {
    Create,
    Complete,
}

#[rstest]
#[case::create_llama_default(
    PartnerUrlKind::Create,
    Partner::Llama,
    None,
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::create_llama_stream_ignored(
    PartnerUrlKind::Create,
    Partner::Llama,
    Some(true),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::create_llama_custom_host(
    PartnerUrlKind::Create,
    Partner::Llama,
    None,
    Some("https://proxy.example.com"),
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::create_llama_custom_version_root(
    PartnerUrlKind::Create,
    Partner::Llama,
    None,
    Some("https://proxy.example.com/v1"),
    "https://proxy.example.com/v1/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::create_mistral_raw(
    PartnerUrlKind::Create,
    Partner::MistralAi,
    Some(false),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:rawPredict"
)]
#[case::create_mistral_stream(
    PartnerUrlKind::Create,
    Partner::MistralAi,
    Some(true),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:streamRawPredict"
)]
#[case::create_mistral_absent_stream(
    PartnerUrlKind::Create,
    Partner::MistralAi,
    None,
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:rawPredict"
)]
#[case::create_ai21_raw(
    PartnerUrlKind::Create,
    Partner::Ai21,
    Some(false),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/ai21/models/gemini-2.5-flash:rawPredict"
)]
#[case::create_ai21_stream(
    PartnerUrlKind::Create,
    Partner::Ai21,
    Some(true),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/ai21/models/gemini-2.5-flash:streamRawPredict"
)]
#[case::create_claude_raw(
    PartnerUrlKind::Create,
    Partner::Claude,
    Some(false),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/anthropic/models/gemini-2.5-flash:rawPredict"
)]
#[case::create_claude_stream(
    PartnerUrlKind::Create,
    Partner::Claude,
    Some(true),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/anthropic/models/gemini-2.5-flash:streamRawPredict"
)]
#[case::create_claude_custom_full(
    PartnerUrlKind::Create,
    Partner::Claude,
    Some(false),
    Some("https://proxy.example.com/foo"),
    "https://proxy.example.com/foo/v1/projects/proj/locations/us-central1/publishers/anthropic/models/gemini-2.5-flash:rawPredict"
)]
#[case::create_empty_base(
    PartnerUrlKind::Create,
    Partner::Claude,
    Some(false),
    Some(""),
    "/v1/projects/proj/locations/us-central1/publishers/anthropic/models/gemini-2.5-flash:rawPredict"
)]
#[case::complete_llama_default(
    PartnerUrlKind::Complete,
    Partner::Llama,
    None,
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::complete_llama_custom_host(
    PartnerUrlKind::Complete,
    Partner::Llama,
    None,
    Some("https://proxy.example.com"),
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::complete_llama_custom_version_root(
    PartnerUrlKind::Complete,
    Partner::Llama,
    None,
    Some("https://proxy.example.com/v1"),
    "https://proxy.example.com/v1/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::complete_llama_custom_full(
    PartnerUrlKind::Complete,
    Partner::Llama,
    None,
    Some("https://proxy.example.com/foo"),
    "https://proxy.example.com/foo/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::complete_mistral_default_raw(
    PartnerUrlKind::Complete,
    Partner::MistralAi,
    Some(false),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:rawPredict"
)]
#[case::complete_mistral_default_stream(
    PartnerUrlKind::Complete,
    Partner::MistralAi,
    Some(true),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:streamRawPredict"
)]
#[case::complete_mistral_custom_host_stream(
    PartnerUrlKind::Complete,
    Partner::MistralAi,
    Some(true),
    Some("https://proxy.example.com"),
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:streamRawPredict?alt=sse"
)]
#[case::complete_mistral_custom_version_root(
    PartnerUrlKind::Complete,
    Partner::MistralAi,
    Some(false),
    Some("https://proxy.example.com/v1"),
    "https://proxy.example.com/v1/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:rawPredict"
)]
#[case::complete_mistral_custom_full(
    PartnerUrlKind::Complete,
    Partner::MistralAi,
    Some(false),
    Some("https://proxy.example.com/foo"),
    "https://proxy.example.com/foo:rawPredict"
)]
#[case::complete_ai21_default_raw(
    PartnerUrlKind::Complete,
    Partner::Ai21,
    Some(false),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/ai21/models/gemini-2.5-flash:rawPredict"
)]
#[case::complete_ai21_custom_host(
    PartnerUrlKind::Complete,
    Partner::Ai21,
    Some(false),
    Some("https://proxy.example.com"),
    "https://proxy.example.com/v1beta1/projects/proj/locations/us-central1/publishers/ai21/models/gemini-2.5-flash:rawPredict"
)]
#[case::complete_claude_custom_version_root_stream(
    PartnerUrlKind::Complete,
    Partner::Claude,
    Some(true),
    Some("https://proxy.example.com/v1"),
    "https://proxy.example.com/v1/v1/projects/proj/locations/us-central1/publishers/anthropic/models/gemini-2.5-flash:streamRawPredict?alt=sse"
)]
fn partner_url_cases(
    #[case] kind: PartnerUrlKind,
    #[case] partner: Partner,
    #[case] stream: Option<bool>,
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    let streaming = stream == Some(true);
    let url = match kind {
        PartnerUrlKind::Create => {
            create_vertex_url(&partner_input!(partner, streaming, api_base)).unwrap()
        }
        PartnerUrlKind::Complete => {
            get_complete_vertex_url(&catalog(), &partner_request!(partner, streaming, api_base))
                .unwrap()
        }
    };
    assert_eq!(url, expected);
}

#[test]
fn create_vertex_url_validates_the_location_only_without_a_custom_base() {
    let mut with_base = partner_input!(Partner::Llama, false, Some("https://h"));
    with_base.location = "bad loc!";
    assert_eq!(
        create_vertex_url(&with_base).unwrap(),
        "https://h/v1/projects/proj/locations/bad loc!/endpoints/openapi/chat/completions"
    );
    let mut without_base = partner_input!(Partner::Llama, false, None);
    without_base.location = "bad loc!";
    let error = create_vertex_url(&without_base).unwrap_err();
    assert!(matches!(
        error,
        EndpointTargetError::VertexUrl(VertexUrlError::InvalidLocation)
    ));
    assert_eq!(
        create_vertex_url(&partner_input!(Partner::Llama, false, None)).unwrap(),
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
    );
}

#[test]
fn complete_vertex_url_falls_back_to_the_project_id() {
    let mut request = partner_request!(Partner::MistralAi, false, None);
    request.vertex_project = None;
    let url = get_complete_vertex_url(&catalog(), &request).unwrap();
    assert_eq!(
        url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/defproj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:rawPredict"
    );
}

#[test]
fn complete_vertex_url_requires_a_project() {
    let mut request = partner_request!(Partner::MistralAi, false, None);
    request.vertex_project = Some("");
    request.project_id = "";
    assert!(matches!(
        get_complete_vertex_url(&catalog(), &request).unwrap_err(),
        EndpointTargetError::Project
    ));
}

#[test]
fn complete_vertex_url_resolves_a_supported_region() {
    let mut request = partner_request!(Partner::MistralAi, false, None);
    request.location = Some("europe-west1");
    request.model = "deepseek-ai/deepseek-v3.1-maas";
    let url = get_complete_vertex_url(&catalog(), &request).unwrap();
    assert_eq!(
        url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/deepseek-ai/deepseek-v3.1-maas:rawPredict"
    );
}
