#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::endpoint_target::error::EndpointTargetError;
use crate::endpoint_target::graft::graft_default_vertex_path;
use crate::endpoint_target::headers::set_headers;
use crate::endpoint_target::partner::{Partner, PartnerUrlInput, create_vertex_url};
use crate::endpoint_target::passthrough::{
    api_version_for_route, construct_target_url, get_vertex_location_from_url,
    get_vertex_model_id_from_url, get_vertex_project_id_from_url, replace_route_project_location,
    with_api_version,
};
use crate::endpoint_target::region::{DEFAULT_LOCATION, select_region};
use crate::endpoint_target::target::{
    AuthHeader, CustomProxyInput, EndpointTarget, PartnerRequest, TargetProvider, TokenUrlInput,
    check_custom_proxy, get_api_base, get_complete_vertex_url, get_token_and_url,
};
use crate::model_urls::error::VertexUrlError;
use crate::model_urls::{ApiVersion, UrlMode};

const PROJECT: &str = "proj";
const LOCATION: &str = "us-central1";
const MODEL: &str = "gemini-2.5-flash";
const NATIVE_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent";

fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn exposed(value: &SecretString) -> &str {
    value.expose_secret()
}

fn proxy_input(api_base: Option<&str>, stream: Option<bool>) -> CustomProxyInput<'_> {
    CustomProxyInput {
        api_base,
        provider: TargetProvider::Vertex,
        gemini_api_key: None,
        endpoint: "generateContent",
        stream,
        auth_header: None,
        url: NATIVE_URL,
        model: Some(MODEL),
        project: Some(PROJECT),
        location: Some(LOCATION),
        api_version: Some(ApiVersion::V1),
        use_psc_endpoint_format: false,
    }
}

fn token_input(auth_token: Option<&SecretString>) -> TokenUrlInput<'_> {
    TokenUrlInput {
        provider: TargetProvider::Vertex,
        model: MODEL,
        auth_token,
        gemini_api_key: None,
        project: Some(PROJECT),
        location: Some(LOCATION),
        stream: None,
        api_base: None,
        use_v1beta1_features: false,
        mode: UrlMode::Chat,
        use_psc_endpoint_format: false,
    }
}

fn partner_request(
    partner: Partner,
    stream: Option<bool>,
    custom_api_base: Option<&str>,
) -> PartnerRequest<'_> {
    PartnerRequest {
        custom_api_base,
        location: Some(LOCATION),
        vertex_project: Some(PROJECT),
        project_id: "defproj",
        partner,
        stream,
        model: MODEL,
    }
}

fn partner_input(
    partner: Partner,
    stream: Option<bool>,
    api_base: Option<&str>,
) -> PartnerUrlInput<'_> {
    PartnerUrlInput {
        location: LOCATION,
        project: PROJECT,
        partner,
        stream,
        model: MODEL,
        api_base,
    }
}

#[derive(Debug, Clone, Copy)]
enum RegionCase {
    AbsentWithoutRegions,
    AbsentWithRegions,
    ConfiguredSupported,
    ConfiguredUnsupported,
    PrefixedModel,
    KnownModelWithoutRegions,
}

#[rstest]
#[case::absent_without_regions(RegionCase::AbsentWithoutRegions)]
#[case::absent_with_regions(RegionCase::AbsentWithRegions)]
#[case::configured_supported(RegionCase::ConfiguredSupported)]
#[case::configured_unsupported(RegionCase::ConfiguredUnsupported)]
#[case::prefixed_model(RegionCase::PrefixedModel)]
#[case::known_model_without_regions(RegionCase::KnownModelWithoutRegions)]
fn region_cases(#[case] scenario: RegionCase) {
    let catalog = catalog();
    match scenario {
        RegionCase::AbsentWithoutRegions => {
            let selection = select_region(&catalog, MODEL, None);
            assert_eq!(selection.region, DEFAULT_LOCATION);
            assert_eq!(selection.unsupported, None);
        }
        RegionCase::AbsentWithRegions => {
            let selection = select_region(&catalog, "deepseek-ai/deepseek-v3.1-maas", None);
            assert_eq!(selection.region, "us-central1");
            assert_eq!(selection.unsupported, None);
        }
        RegionCase::ConfiguredSupported => {
            let selection = select_region(
                &catalog,
                "qwen/qwen3-235b-a22b-instruct-2507-maas",
                Some("us-south1"),
            );
            assert_eq!(selection.region, "us-south1");
            assert_eq!(selection.unsupported, None);
        }
        RegionCase::ConfiguredUnsupported => {
            let selection = select_region(
                &catalog,
                "qwen/qwen3-235b-a22b-instruct-2507-maas",
                Some("europe-west1"),
            );
            assert_eq!(selection.region, "global");
            let unsupported = selection.unsupported.expect("the region is unsupported");
            assert_eq!(unsupported.configured, "europe-west1");
            assert_eq!(unsupported.supported, vec!["global", "us-south1"]);
        }
        RegionCase::PrefixedModel => {
            let selection = select_region(
                &catalog,
                "vertex_ai/deepseek-ai/deepseek-v3.1-maas",
                Some("global"),
            );
            assert_eq!(selection.region, "us-central1");
            assert!(selection.unsupported.is_some());
        }
        RegionCase::KnownModelWithoutRegions => {
            let selection = select_region(&catalog, "text-embedding-005", Some("europe-west1"));
            assert_eq!(selection.region, "europe-west1");
            assert_eq!(selection.unsupported, None);
        }
    }
}

#[rstest]
#[case::custom(
    Some("https://proxy.example.com/v1"),
    Some("europe-west1"),
    "https://proxy.example.com/v1"
)]
#[case::empty_custom(
    Some(""),
    Some("europe-west1"),
    "https://europe-west1-aiplatform.googleapis.com"
)]
#[case::absent_default(Some(""), None, "https://us-central1-aiplatform.googleapis.com")]
#[case::global(None, Some("global"), "https://aiplatform.googleapis.com")]
#[case::geography(None, Some("us"), "https://aiplatform.us.rep.googleapis.com")]
fn api_base_cases(
    #[case] api_base: Option<&str>,
    #[case] location: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(get_api_base(api_base, location).unwrap(), expected);
}

#[test]
fn api_base_rejects_an_invalid_location() {
    let error = get_api_base(None, Some("bad loc!")).unwrap_err();
    assert!(matches!(
        error,
        EndpointTargetError::VertexUrl(VertexUrlError::InvalidLocation)
    ));
}

#[rstest]
#[case::llama_default(
    Partner::Llama,
    None,
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::llama_stream_ignored(
    Partner::Llama,
    Some(true),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::llama_custom_host(
    Partner::Llama,
    None,
    Some("https://proxy.example.com"),
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::llama_custom_version_root(
    Partner::Llama,
    None,
    Some("https://proxy.example.com/v1"),
    "https://proxy.example.com/v1/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::mistral_raw(
    Partner::MistralAi,
    Some(false),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:rawPredict"
)]
#[case::mistral_stream(
    Partner::MistralAi,
    Some(true),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:streamRawPredict"
)]
#[case::mistral_absent_stream(
    Partner::MistralAi,
    None,
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:rawPredict"
)]
#[case::ai21_raw(
    Partner::Ai21,
    Some(false),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/ai21/models/gemini-2.5-flash:rawPredict"
)]
#[case::ai21_stream(
    Partner::Ai21,
    Some(true),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/ai21/models/gemini-2.5-flash:streamRawPredict"
)]
#[case::claude_raw(
    Partner::Claude,
    Some(false),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/anthropic/models/gemini-2.5-flash:rawPredict"
)]
#[case::claude_stream(
    Partner::Claude,
    Some(true),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/anthropic/models/gemini-2.5-flash:streamRawPredict"
)]
#[case::claude_custom_full(
    Partner::Claude,
    Some(false),
    Some("https://proxy.example.com/foo"),
    "https://proxy.example.com/foo/v1/projects/proj/locations/us-central1/publishers/anthropic/models/gemini-2.5-flash:rawPredict"
)]
#[case::empty_base_is_used_as_given(
    Partner::Claude,
    Some(false),
    Some(""),
    "/v1/projects/proj/locations/us-central1/publishers/anthropic/models/gemini-2.5-flash:rawPredict"
)]
fn create_vertex_url_cases(
    #[case] partner: Partner,
    #[case] stream: Option<bool>,
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    let input = partner_input(partner, stream, api_base);
    assert_eq!(create_vertex_url(&input).unwrap(), expected);
}

#[test]
fn create_vertex_url_validates_the_location_only_without_a_custom_base() {
    let input = partner_input(Partner::Llama, None, None);
    let mut invalid = partner_input(Partner::Llama, None, Some("https://h"));
    invalid.location = "bad loc!";
    let error = create_vertex_url(&invalid).unwrap_err();
    assert!(matches!(
        error,
        EndpointTargetError::VertexUrl(VertexUrlError::InvalidLocation)
    ));
    assert_eq!(
        create_vertex_url(&input).unwrap(),
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
    );
}

#[rstest]
#[case::llama_default(
    Partner::Llama,
    None,
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::llama_custom_host(
    Partner::Llama,
    None,
    Some("https://proxy.example.com"),
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::llama_custom_version_root(
    Partner::Llama,
    None,
    Some("https://proxy.example.com/v1"),
    "https://proxy.example.com/v1/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::llama_custom_full(
    Partner::Llama,
    None,
    Some("https://proxy.example.com/foo"),
    "https://proxy.example.com/foo/v1/projects/proj/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::mistral_default_raw(
    Partner::MistralAi,
    Some(false),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:rawPredict"
)]
#[case::mistral_default_stream(
    Partner::MistralAi,
    Some(true),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:streamRawPredict"
)]
#[case::mistral_custom_host_stream(
    Partner::MistralAi,
    Some(true),
    Some("https://proxy.example.com"),
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:streamRawPredict?alt=sse"
)]
#[case::mistral_custom_version_root(
    Partner::MistralAi,
    Some(false),
    Some("https://proxy.example.com/v1"),
    "https://proxy.example.com/v1/v1/projects/proj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:rawPredict"
)]
#[case::mistral_custom_full(
    Partner::MistralAi,
    Some(false),
    Some("https://proxy.example.com/foo"),
    "https://proxy.example.com/foo:rawPredict"
)]
#[case::ai21_default_raw(
    Partner::Ai21,
    Some(false),
    None,
    "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/ai21/models/gemini-2.5-flash:rawPredict"
)]
#[case::ai21_custom_host(
    Partner::Ai21,
    Some(false),
    Some("https://proxy.example.com"),
    "https://proxy.example.com/v1beta1/projects/proj/locations/us-central1/publishers/ai21/models/gemini-2.5-flash:rawPredict"
)]
#[case::claude_custom_version_root_stream(
    Partner::Claude,
    Some(true),
    Some("https://proxy.example.com/v1"),
    "https://proxy.example.com/v1/v1beta1/projects/proj/locations/us-central1/publishers/anthropic/models/gemini-2.5-flash:streamRawPredict?alt=sse"
)]
fn complete_vertex_url_cases(
    #[case] partner: Partner,
    #[case] stream: Option<bool>,
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    let request = partner_request(partner, stream, api_base);
    assert_eq!(
        get_complete_vertex_url(&catalog(), &request).unwrap(),
        expected
    );
}

#[test]
fn complete_vertex_url_falls_back_to_the_project_id() {
    let request = PartnerRequest {
        vertex_project: None,
        ..partner_request(Partner::MistralAi, Some(false), None)
    };
    let url = get_complete_vertex_url(&catalog(), &request).unwrap();
    assert_eq!(
        url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/defproj/locations/us-central1/publishers/mistralai/models/gemini-2.5-flash:rawPredict"
    );
}

#[test]
fn complete_vertex_url_requires_a_project() {
    let request = PartnerRequest {
        vertex_project: Some(""),
        project_id: "",
        ..partner_request(Partner::MistralAi, Some(false), None)
    };
    assert!(matches!(
        get_complete_vertex_url(&catalog(), &request).unwrap_err(),
        EndpointTargetError::Project
    ));
}

#[test]
fn complete_vertex_url_resolves_a_supported_region() {
    let request = PartnerRequest {
        location: Some("europe-west1"),
        model: "deepseek-ai/deepseek-v3.1-maas",
        ..partner_request(Partner::MistralAi, Some(false), None)
    };
    let url = get_complete_vertex_url(&catalog(), &request).unwrap();
    assert_eq!(
        url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/mistralai/models/deepseek-ai/deepseek-v3.1-maas:rawPredict"
    );
}

#[derive(Debug, Clone, Copy)]
enum TokenCase {
    VertexDefault,
    VertexBeta,
    VertexStream,
    VertexCustomHost,
    VertexCustomVersionRoot,
    VertexCustomFull,
    VertexNoAuth,
    VertexEmbedding,
    VertexCountTokensBeta,
    VertexRegionFallback,
    GeminiDefault,
    GeminiNewerStream,
    GeminiCustomHost,
    GeminiIgnoresAuthToken,
}

#[rstest]
#[case::vertex_default(TokenCase::VertexDefault)]
#[case::vertex_beta(TokenCase::VertexBeta)]
#[case::vertex_stream(TokenCase::VertexStream)]
#[case::vertex_custom_host(TokenCase::VertexCustomHost)]
#[case::vertex_custom_version_root(TokenCase::VertexCustomVersionRoot)]
#[case::vertex_custom_full(TokenCase::VertexCustomFull)]
#[case::vertex_no_auth(TokenCase::VertexNoAuth)]
#[case::vertex_embedding(TokenCase::VertexEmbedding)]
#[case::vertex_count_tokens_beta(TokenCase::VertexCountTokensBeta)]
#[case::vertex_region_fallback(TokenCase::VertexRegionFallback)]
#[case::gemini_default(TokenCase::GeminiDefault)]
#[case::gemini_newer_stream(TokenCase::GeminiNewerStream)]
#[case::gemini_custom_host(TokenCase::GeminiCustomHost)]
#[case::gemini_ignores_auth_token(TokenCase::GeminiIgnoresAuthToken)]
fn token_and_url_cases(#[case] scenario: TokenCase) {
    let auth = secret("tok");
    let key = secret("gk");
    let vertex_expected = |url: &str| {
        let target = get_token_and_url(&catalog(), &token_input(Some(&auth))).unwrap();
        assert_eq!(target.url, url);
        assert_eq!(token_of(&target), Some("tok"));
    };
    match scenario {
        TokenCase::VertexDefault => vertex_expected(
            "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent",
        ),
        TokenCase::VertexBeta => {
            let input = TokenUrlInput {
                use_v1beta1_features: true,
                ..token_input(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(
                target.url,
                "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
            );
        }
        TokenCase::VertexStream => {
            let input = TokenUrlInput {
                stream: Some(true),
                ..token_input(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(
                target.url,
                "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
            );
            assert_eq!(token_of(&target), Some("tok"));
        }
        TokenCase::VertexCustomHost => {
            let input = TokenUrlInput {
                api_base: Some("https://proxy.example.com"),
                ..token_input(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(
                target.url,
                "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
            );
        }
        TokenCase::VertexCustomVersionRoot => {
            let input = TokenUrlInput {
                api_base: Some("https://proxy.example.com/v1"),
                ..token_input(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(
                target.url,
                "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
            );
        }
        TokenCase::VertexCustomFull => {
            let input = TokenUrlInput {
                api_base: Some("https://proxy.example.com/foo"),
                ..token_input(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(target.url, "https://proxy.example.com/foo:generateContent");
        }
        TokenCase::VertexNoAuth => {
            let target = get_token_and_url(&catalog(), &token_input(None)).unwrap();
            assert!(target.auth.is_none());
        }
        TokenCase::VertexEmbedding => {
            let input = TokenUrlInput {
                model: "text-embedding-005",
                mode: UrlMode::Embedding,
                location: Some("global"),
                ..token_input(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(
                target.url,
                "https://aiplatform.googleapis.com/v1/projects/proj/locations/global/publishers/google/models/text-embedding-005:predict"
            );
        }
        TokenCase::VertexCountTokensBeta => {
            let input = TokenUrlInput {
                mode: UrlMode::CountTokens,
                use_v1beta1_features: true,
                ..token_input(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(
                target.url,
                "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:countTokens"
            );
        }
        TokenCase::VertexRegionFallback => {
            let input = TokenUrlInput {
                model: "deepseek-ai/deepseek-v3.1-maas",
                location: Some("europe-west1"),
                ..token_input(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(
                target.url,
                "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/deepseek-ai/deepseek-v3.1-maas:generateContent"
            );
        }
        TokenCase::GeminiDefault => {
            let input = TokenUrlInput {
                provider: TargetProvider::Gemini,
                auth_token: Some(&auth),
                gemini_api_key: Some(&key),
                ..token_input(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(
                target.url,
                "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
            );
            assert_eq!(header_of(&target, "x-goog-api-key"), Some("gk"));
        }
        TokenCase::GeminiNewerStream => {
            let input = TokenUrlInput {
                provider: TargetProvider::Gemini,
                model: "gemini-3-pro-preview",
                stream: Some(true),
                gemini_api_key: Some(&key),
                project: None,
                location: None,
                ..token_input(None)
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(
                target.url,
                "https://generativelanguage.googleapis.com/v1alpha/models/gemini-3-pro-preview:streamGenerateContent?alt=sse"
            );
            assert_eq!(header_of(&target, "x-goog-api-key"), Some("gk"));
        }
        TokenCase::GeminiCustomHost => {
            let input = TokenUrlInput {
                provider: TargetProvider::Gemini,
                api_base: Some("https://custom.example.com"),
                gemini_api_key: Some(&key),
                ..token_input(None)
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(
                target.url,
                "https://custom.example.com/models/gemini-2.5-flash:generateContent"
            );
            assert_eq!(header_of(&target, "x-goog-api-key"), Some("gk"));
        }
        TokenCase::GeminiIgnoresAuthToken => {
            let input = TokenUrlInput {
                provider: TargetProvider::Gemini,
                gemini_api_key: Some(&key),
                ..token_input(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert!(matches!(target.auth, Some(AuthHeader::Headers(_))));
            assert_eq!(header_of(&target, "x-goog-api-key"), Some("gk"));
        }
    }
}

#[test]
fn token_and_url_requires_a_gemini_key() {
    let input = TokenUrlInput {
        provider: TargetProvider::Gemini,
        ..token_input(None)
    };
    assert!(matches!(
        get_token_and_url(&catalog(), &input).unwrap_err(),
        EndpointTargetError::GeminiApiKey
    ));
}

#[test]
fn token_and_url_requires_a_project_for_vertex() {
    let input = TokenUrlInput {
        project: None,
        ..token_input(None)
    };
    assert!(matches!(
        get_token_and_url(&catalog(), &input).unwrap_err(),
        EndpointTargetError::VertexUrl(VertexUrlError::ProjectRequired)
    ));
}

#[test]
fn token_and_url_rejects_batch_embedding() {
    let input = TokenUrlInput {
        mode: UrlMode::BatchEmbedding,
        ..token_input(None)
    };
    assert!(matches!(
        get_token_and_url(&catalog(), &input).unwrap_err(),
        EndpointTargetError::VertexUrl(VertexUrlError::UnsupportedMode { .. })
    ));
}

#[rstest]
#[case::absent(
    None,
    false,
    NATIVE_URL,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::empty(
    Some(""),
    false,
    NATIVE_URL,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::host_only(
    Some("https://proxy.example.com"),
    false,
    NATIVE_URL,
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::host_only_slash(
    Some("https://proxy.example.com/"),
    false,
    NATIVE_URL,
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::version_root_v1(
    Some("https://proxy.example.com/v1"),
    false,
    NATIVE_URL,
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::version_root_v1beta1(
    Some("https://proxy.example.com/v1beta1"),
    false,
    NATIVE_URL,
    "https://proxy.example.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::full_model(
    Some("https://proxy.example.com/v1/models/gemini-2.5-flash"),
    false,
    NATIVE_URL,
    "https://proxy.example.com/v1/models/gemini-2.5-flash:generateContent"
)]
#[case::full_model_slash(
    Some("https://proxy.example.com/foo/"),
    false,
    NATIVE_URL,
    "https://proxy.example.com/foo/:generateContent"
)]
#[case::version_root_without_projects(
    Some("https://proxy.example.com/v1"),
    false,
    "https://h.example.com/v1/foo:generateContent",
    "https://proxy.example.com/v1:generateContent"
)]
fn custom_proxy_graft_cases(
    #[case] api_base: Option<&str>,
    #[case] stream: bool,
    #[case] url: &str,
    #[case] expected: &str,
) {
    let input = CustomProxyInput {
        url,
        ..proxy_input(api_base, Some(stream))
    };
    let target = check_custom_proxy(&input).unwrap();
    assert_eq!(target.url, expected);
    assert!(target.auth.is_none());
}

#[rstest]
#[case::version_root_stream(
    Some("https://proxy.example.com/v1"),
    true,
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent?alt=sse"
)]
#[case::stream_with_query(
    Some("https://proxy.example.com/v1?x=1"),
    true,
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent?x=1&alt=sse"
)]
#[case::query_without_stream(
    Some("https://proxy.example.com/v1?x=1&y=2"),
    false,
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent?x=1&y=2"
)]
#[case::absent_base_ignores_stream(
    None,
    true,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
fn custom_proxy_stream_cases(
    #[case] api_base: Option<&str>,
    #[case] stream: bool,
    #[case] expected: &str,
) {
    let target = check_custom_proxy(&proxy_input(api_base, Some(stream))).unwrap();
    assert_eq!(target.url, expected);
}

#[test]
fn custom_proxy_leaves_a_native_query_alone_without_a_custom_base() {
    let url = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse";
    let input = CustomProxyInput {
        url,
        stream: Some(true),
        ..proxy_input(None, None)
    };
    let target = check_custom_proxy(&input).unwrap();
    assert_eq!(target.url, url);
}

#[test]
fn custom_proxy_merges_a_full_model_base_query() {
    let input = CustomProxyInput {
        api_base: Some("https://proxy.example.com/foo?x=1"),
        ..proxy_input(None, Some(true))
    };
    let target = check_custom_proxy(&input).unwrap();
    assert_eq!(
        target.url,
        "https://proxy.example.com/foo?x=1:generateContent&alt=sse"
    );
}

#[test]
fn custom_proxy_appends_the_stream_query_before_an_existing_fragment() {
    let input = CustomProxyInput {
        api_base: Some("https://proxy.example.com/foo#frag"),
        url: "https://h.example.com/v1/foo:generateContent",
        ..proxy_input(None, Some(true))
    };
    assert_eq!(
        check_custom_proxy(&input).unwrap().url,
        "https://proxy.example.com/foo?alt=sse#frag:generateContent"
    );
}

#[test]
fn custom_proxy_passes_the_auth_token_through() {
    let auth = AuthHeader::Token(secret("tok"));
    let input = CustomProxyInput {
        auth_header: Some(&auth),
        ..proxy_input(Some("https://proxy.example.com"), None)
    };
    let target = check_custom_proxy(&input).unwrap();
    assert_eq!(token_of(&target), Some("tok"));
}

#[rstest]
#[case::absent_version(
    Some("https://psc.example.com"),
    None,
    MODEL,
    "https://psc.example.com/v1/projects/proj/locations/us-central1/endpoints/gemini-2.5-flash:generateContent"
)]
#[case::default_version(
    Some("https://psc.example.com"),
    Some("v1"),
    MODEL,
    "https://psc.example.com/v1/projects/proj/locations/us-central1/endpoints/gemini-2.5-flash:generateContent"
)]
#[case::trailing_slash(
    Some("https://psc.example.com/"),
    Some("v1"),
    MODEL,
    "https://psc.example.com/v1/projects/proj/locations/us-central1/endpoints/gemini-2.5-flash:generateContent"
)]
#[case::explicit_v1beta1(
    Some("https://psc.example.com"),
    Some("v1beta1"),
    MODEL,
    "https://psc.example.com/v1beta1/projects/proj/locations/us-central1/endpoints/gemini-2.5-flash:generateContent"
)]
#[case::known_prefix(
    Some("https://psc.example.com"),
    Some("v1"),
    "bge/1234",
    "https://psc.example.com/v1/projects/proj/locations/us-central1/endpoints/1234:generateContent"
)]
#[case::unknown_prefix(
    Some("https://psc.example.com"),
    Some("v1"),
    "custom/publisher-model",
    "https://psc.example.com/v1/projects/proj/locations/us-central1/endpoints/custom/publisher-model:generateContent"
)]
fn psc_target_cases(
    #[case] api_base: Option<&str>,
    #[case] version: Option<&str>,
    #[case] model: &str,
    #[case] expected: &str,
) {
    let input = CustomProxyInput {
        api_base,
        model: Some(model),
        api_version: version.map(version_of),
        use_psc_endpoint_format: true,
        ..proxy_input(None, None)
    };
    assert_eq!(check_custom_proxy(&input).unwrap().url, expected);
}

#[test]
fn psc_target_streams_with_alt_sse() {
    let input = CustomProxyInput {
        api_base: Some("https://psc.example.com"),
        stream: Some(true),
        use_psc_endpoint_format: true,
        ..proxy_input(None, None)
    };
    assert_eq!(
        check_custom_proxy(&input).unwrap().url,
        "https://psc.example.com/v1/projects/proj/locations/us-central1/endpoints/gemini-2.5-flash:generateContent?alt=sse"
    );
}

#[rstest]
#[case::missing_project(None, Some(LOCATION), Some(MODEL))]
#[case::missing_location(Some(PROJECT), None, Some(MODEL))]
#[case::missing_model(Some(PROJECT), Some(LOCATION), None)]
#[case::empty_project(Some(""), Some(LOCATION), Some(MODEL))]
#[case::empty_location(Some(PROJECT), Some(""), Some(MODEL))]
#[case::empty_model(Some(PROJECT), Some(LOCATION), Some(""))]
fn psc_target_requires_every_input(
    #[case] project: Option<&str>,
    #[case] location: Option<&str>,
    #[case] model: Option<&str>,
) {
    let input = CustomProxyInput {
        api_base: Some("https://psc.example.com"),
        project,
        location,
        model,
        use_psc_endpoint_format: true,
        ..proxy_input(None, None)
    };
    assert!(matches!(
        check_custom_proxy(&input).unwrap_err(),
        EndpointTargetError::PscInputs
    ));
}

#[test]
fn gemini_custom_base_builds_the_model_path_and_key_header() {
    let key = secret("gk");
    let input = CustomProxyInput {
        api_base: Some("https://custom.example.com"),
        provider: TargetProvider::Gemini,
        gemini_api_key: Some(&key),
        ..proxy_input(None, None)
    };
    let target = check_custom_proxy(&input).unwrap();
    assert_eq!(
        target.url,
        "https://custom.example.com/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(header_of(&target, "x-goog-api-key"), Some("gk"));
}

#[test]
fn gemini_custom_base_streams_with_alt_sse() {
    let key = secret("gk");
    let input = CustomProxyInput {
        api_base: Some("https://custom.example.com"),
        provider: TargetProvider::Gemini,
        gemini_api_key: Some(&key),
        endpoint: "streamGenerateContent",
        stream: Some(true),
        ..proxy_input(None, None)
    };
    assert_eq!(
        check_custom_proxy(&input).unwrap().url,
        "https://custom.example.com/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
    );
}

#[test]
fn gemini_custom_base_requires_a_model() {
    let key = secret("gk");
    let input = CustomProxyInput {
        api_base: Some("https://custom.example.com"),
        provider: TargetProvider::Gemini,
        gemini_api_key: Some(&key),
        model: None,
        ..proxy_input(None, None)
    };
    assert!(matches!(
        check_custom_proxy(&input).unwrap_err(),
        EndpointTargetError::GeminiModel
    ));
}

#[test]
fn gemini_custom_base_requires_a_key() {
    let input = CustomProxyInput {
        api_base: Some("https://custom.example.com"),
        provider: TargetProvider::Gemini,
        ..proxy_input(None, None)
    };
    assert!(matches!(
        check_custom_proxy(&input).unwrap_err(),
        EndpointTargetError::GeminiApiKey
    ));
}

#[test]
fn gemini_without_a_custom_base_returns_the_passed_auth() {
    let auth = AuthHeader::Token(secret("tok"));
    let input = CustomProxyInput {
        provider: TargetProvider::Gemini,
        auth_header: Some(&auth),
        ..proxy_input(None, None)
    };
    let target = check_custom_proxy(&input).unwrap();
    assert_eq!(target.url, NATIVE_URL);
    assert_eq!(token_of(&target), Some("tok"));
}

#[rstest]
#[case::only_content_type(None, &[], &[])]
#[case::bearer(Some("tok"), &[], &[("Authorization", "Bearer tok")])]
#[case::empty_bearer(Some(""), &[], &[("Authorization", "Bearer ")])]
#[case::content_type_override(None, &[("Content-Type", "text/plain")], &[("Content-Type", "text/plain")])]
#[case::authorization_override(Some("tok"), &[("Authorization", "Custom x")], &[("Authorization", "Custom x")])]
#[case::extra_header(Some("tok"), &[("X-Extra", "1")], &[("Authorization", "Bearer tok"), ("X-Extra", "1")])]
#[case::lowercase_content_type(Some("tok"), &[("content-type", "text/plain")], &[("Authorization", "Bearer tok"), ("content-type", "text/plain")])]
fn set_headers_cases(
    #[case] auth: Option<&str>,
    #[case] extras: &[(&str, &str)],
    #[case] expected_extras: &[(&str, &str)],
) {
    let token = auth.map(secret);
    let extra_headers: BTreeMap<String, String> = extras
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let headers = set_headers(token.as_ref(), &extra_headers);
    let exposed_headers: BTreeMap<String, String> = headers
        .iter()
        .map(|(name, value)| (name.clone(), exposed(value).to_owned()))
        .collect();
    assert_eq!(exposed_headers, expected_headers(expected_extras));
}

fn expected_headers(extras: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([("Content-Type".to_owned(), "application/json".to_owned())]);
    for (name, value) in extras {
        headers.insert((*name).to_owned(), (*value).to_owned());
    }
    headers
}

#[test]
fn endpoint_target_debug_redacts_credentials() {
    let auth = AuthHeader::Token(secret("tok-secret"));
    let target = EndpointTarget {
        url: "https://example.com".to_owned(),
        auth: Some(auth),
    };
    let debug = format!("{target:?}");
    assert!(!debug.contains("tok-secret"));
    assert_eq!(token_of(&target), Some("tok-secret"));
}

#[rstest]
#[case::complete_url(
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj-1/locations/us-central1/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
    Some("proj-1"),
    Some("us-central1"),
    Some("gemini-2.5-flash")
)]
#[case::cached_content(
    "https://h.example.com/v1beta1/projects/p/locations/l/cachedContents/abc",
    Some("p"),
    Some("l"),
    None
)]
#[case::model_with_slash(
    "https://h.example.com/x/models/a/b:generateContent",
    None,
    None,
    Some("a/b")
)]
#[case::no_markers("https://h.example.com/nothing", None, None, None)]
#[case::empty_segments(
    "https://h.example.com/projects//locations//models/:x",
    None,
    None,
    None
)]
#[case::first_match(
    "https://h.example.com/projects/p/locations/l/x/projects/q/locations/r",
    Some("p"),
    Some("l"),
    None
)]
fn passthrough_extraction_cases(
    #[case] url: &str,
    #[case] project: Option<&str>,
    #[case] location: Option<&str>,
    #[case] model: Option<&str>,
) {
    assert_eq!(get_vertex_project_id_from_url(url), project);
    assert_eq!(get_vertex_location_from_url(url), location);
    assert_eq!(get_vertex_model_id_from_url(url), model);
}

#[rstest]
#[case::single("/projects/a/locations/b/x", "/projects/p/locations/l/x")]
#[case::multiple(
    "/projects/a/locations/b/x/projects/c/locations/d/y",
    "/projects/p/locations/l/x/projects/p/locations/l/y"
)]
#[case::no_trailing_slash("/projects/a/locations/b", "/projects/a/locations/b")]
#[case::empty_location("/projects/a/locations//x", "/projects/a/locations//x")]
#[case::not_a_route("/x/projects/a/locations/b/", "/x/projects/p/locations/l/")]
fn replace_route_project_location_cases(#[case] route: &str, #[case] expected: &str) {
    assert_eq!(
        replace_route_project_location(route, "p", "l"),
        expected.to_owned()
    );
}

#[rstest]
#[case::plain("/publishers/google/models/m:generateContent", ApiVersion::V1)]
#[case::cached_content("/projects/p/locations/l/cachedContent", ApiVersion::V1Beta1)]
#[case::cached_contents("/projects/p/locations/l/cachedContents", ApiVersion::V1Beta1)]
#[case::case_sensitive("/projects/p/locations/l/CachedContent", ApiVersion::V1)]
fn api_version_for_route_cases(#[case] route: &str, #[case] expected: ApiVersion) {
    assert_eq!(api_version_for_route(route), expected);
}

#[rstest]
#[case::project_route("/projects/a/x", "/v1/projects/a/x")]
#[case::cached_project_route("/projects/a/cachedContent", "/v1beta1/projects/a/cachedContent")]
#[case::other_route("/publishers/a", "/publishers/a")]
fn with_api_version_cases(#[case] route: &str, #[case] expected: &str) {
    assert_eq!(with_api_version(route), expected);
}

#[rstest]
#[case::replace_both(
    "https://us-central1-aiplatform.googleapis.com",
    "/projects/old/locations/oldloc/publishers/google/models/gemini-2.5-flash:generateContent",
    Some("newproj"),
    Some("newloc"),
    "https://us-central1-aiplatform.googleapis.com/v1/projects/newproj/locations/newloc/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::missing_project_keeps_route(
    "https://us-central1-aiplatform.googleapis.com",
    "/projects/old/locations/oldloc/publishers/google/models/gemini-2.5-flash:generateContent",
    None,
    Some("newloc"),
    "https://us-central1-aiplatform.googleapis.com/v1/projects/old/locations/oldloc/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::empty_location_keeps_route(
    "https://us-central1-aiplatform.googleapis.com",
    "/projects/old/locations/oldloc/publishers/google/models/gemini-2.5-flash:generateContent",
    Some("newproj"),
    Some(""),
    "https://us-central1-aiplatform.googleapis.com/v1/projects/old/locations/oldloc/publishers/google/models/gemini-2.5-flash:generateContent"
)]
#[case::locations_route_without_projects_prefix(
    "https://h.example.com",
    "/some/locations/x",
    Some("p"),
    Some("l"),
    "https://h.example.com/some/locations/x"
)]
#[case::explicit_v1(
    "https://h.example.com",
    "/v1/publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1/projects/p/locations/l/publishers/google/models/m:generateContent"
)]
#[case::explicit_v1beta1(
    "https://h.example.com",
    "/v1beta1/publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1beta1/projects/p/locations/l/publishers/google/models/m:generateContent"
)]
#[case::default_version(
    "https://h.example.com",
    "/publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1/projects/p/locations/l/publishers/google/models/m:generateContent"
)]
#[case::cached_content_default_version(
    "https://h.example.com",
    "/cachedContents",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1beta1/projects/p/locations/l/cachedContents"
)]
#[case::cached_content_route_with_locations(
    "https://h.example.com",
    "/projects/old/locations/old/cachedContents",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1beta1/projects/p/locations/l/cachedContents"
)]
#[case::explicit_version_wins_over_cache(
    "https://h.example.com",
    "/v1/projects/old/locations/old/cachedContents",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1/projects/p/locations/l/cachedContents"
)]
#[case::route_without_leading_slash(
    "https://h.example.com",
    "publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1/projects/p/locations/lpublishers/google/models/m:generateContent"
)]
#[case::base_with_query(
    "https://proxy.example.com/base?x=1",
    "/publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://proxy.example.com/v1/projects/p/locations/l/publishers/google/models/m:generateContent?x=1"
)]
#[case::base_with_trailing_slash(
    "https://h.example.com/",
    "/publishers/google/models/m:generateContent",
    Some("p"),
    Some("l"),
    "https://h.example.com/v1/projects/p/locations/l/publishers/google/models/m:generateContent"
)]
fn construct_target_url_cases(
    #[case] base_url: &str,
    #[case] route: &str,
    #[case] project: Option<&str>,
    #[case] location: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        construct_target_url(base_url, route, project, location).unwrap(),
        expected
    );
}

#[test]
fn construct_target_url_requires_project_and_location_without_a_locations_route() {
    let base = "https://h.example.com";
    let route = "/publishers/google/models/m:generateContent";
    assert!(matches!(
        construct_target_url(base, route, None, Some("l")).unwrap_err(),
        EndpointTargetError::Project
    ));
    assert!(matches!(
        construct_target_url(base, route, Some("p"), None).unwrap_err(),
        EndpointTargetError::Location
    ));
    assert!(matches!(
        construct_target_url(base, route, Some(""), Some("l")).unwrap_err(),
        EndpointTargetError::Project
    ));
}

#[test]
fn construct_target_url_rejects_a_bad_base_or_route() {
    assert!(matches!(
        construct_target_url("not a url", "/x", Some("p"), Some("l")).unwrap_err(),
        EndpointTargetError::BaseUrl { .. }
    ));
    assert!(matches!(
        construct_target_url("ftp://h.example.com", "/x", Some("p"), Some("l")).unwrap_err(),
        EndpointTargetError::BaseScheme
    ));
    assert!(matches!(
        construct_target_url("https://h.example.com", "/x?y=1", Some("p"), Some("l")).unwrap_err(),
        EndpointTargetError::Route
    ));
    assert!(matches!(
        construct_target_url("https://h.example.com", "/x#f", Some("p"), Some("l")).unwrap_err(),
        EndpointTargetError::Route
    ));
    assert!(matches!(
        construct_target_url("https://h.example.com", "/x\ty", Some("p"), Some("l")).unwrap_err(),
        EndpointTargetError::Route
    ));
}

#[test]
fn construct_target_url_normalizes_dot_segments() {
    assert_eq!(
        construct_target_url(
            "https://h.example.com/base",
            "/x/../y",
            Some("p"),
            Some("l")
        )
        .unwrap(),
        "https://h.example.com/v1/projects/p/locations/l/y"
    );
    assert_eq!(
        construct_target_url("https://h.example.com/base", "/../x", Some("p"), Some("l")).unwrap(),
        "https://h.example.com/v1/projects/p/locations/x"
    );
}

#[test]
fn construct_target_url_keeps_the_destination_authority() {
    let url = construct_target_url(
        "https://h.example.com",
        "//evil.example.com/x",
        Some("p"),
        Some("l"),
    )
    .unwrap();
    assert!(url.starts_with("https://h.example.com/"));
    let url_from_absolute_text = construct_target_url(
        "https://h.example.com",
        "https://evil.example.com/x",
        Some("p"),
        Some("l"),
    )
    .unwrap();
    assert!(url_from_absolute_text.starts_with("https://h.example.com/"));
}

#[rstest]
#[case::host_only(
    "https://h.example.com",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent",
    "https://h.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent"
)]
#[case::trailing_slash(
    "https://h.example.com/",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent",
    "https://h.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent"
)]
#[case::version_root(
    "https://h.example.com/v1",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:streamGenerateContent?alt=sse",
    "https://h.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:streamGenerateContent"
)]
#[case::version_root_v1beta1(
    "https://h.example.com/v1beta1",
    "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent",
    "https://h.example.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent"
)]
#[case::query_preserved(
    "https://h.example.com/v1?x=1",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent",
    "https://h.example.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent?x=1"
)]
#[case::base_path(
    "https://h.example.com/base",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/m:generateContent",
    "https://h.example.com/base/projects/proj/locations/us-central1/publishers/google/models/m:generateContent"
)]
fn graft_default_vertex_path_cases(
    #[case] api_base: &str,
    #[case] default_url: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        graft_default_vertex_path(api_base, default_url).unwrap(),
        expected
    );
}

fn token_of(target: &EndpointTarget) -> Option<&str> {
    match &target.auth {
        Some(AuthHeader::Token(token)) => Some(exposed(token)),
        None | Some(AuthHeader::Headers(_)) => None,
    }
}

fn header_of<'target>(target: &'target EndpointTarget, name: &str) -> Option<&'target str> {
    match &target.auth {
        Some(AuthHeader::Headers(headers)) => headers.get(name).map(exposed),
        Some(AuthHeader::Token(_)) | None => None,
    }
}

fn version_of(text: &str) -> ApiVersion {
    if text == "v1beta1" {
        ApiVersion::V1Beta1
    } else {
        ApiVersion::V1
    }
}
