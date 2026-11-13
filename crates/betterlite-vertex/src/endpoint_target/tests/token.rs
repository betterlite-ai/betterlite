#![cfg(test)]

use rstest::rstest;

use crate::endpoint_target::error::EndpointTargetError;
use crate::endpoint_target::target::{
    AuthHeader, TargetProvider, TokenUrlInput, get_token_and_url,
};
use crate::model_urls::error::VertexUrlError;
use crate::model_urls::{ApiVersion, UrlMode};

use super::support::{catalog, header_of, secret, token_of};

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
    match scenario {
        TokenCase::VertexDefault => {
            let target = get_token_and_url(&catalog(), &token_input!(Some(&auth))).unwrap();
            assert_eq!(
                target.url,
                "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
            );
            assert_eq!(token_of(&target), Some("tok"));
        }
        TokenCase::VertexBeta => {
            let input = TokenUrlInput {
                api_version: ApiVersion::V1Beta1,
                ..token_input!(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(
                target.url,
                "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
            );
        }
        TokenCase::VertexStream => {
            let input = TokenUrlInput {
                stream: true,
                ..token_input!(Some(&auth))
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
                ..token_input!(Some(&auth))
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
                ..token_input!(Some(&auth))
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
                ..token_input!(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert_eq!(target.url, "https://proxy.example.com/foo:generateContent");
        }
        TokenCase::VertexNoAuth => {
            let target = get_token_and_url(&catalog(), &token_input!(None)).unwrap();
            assert!(target.auth.is_none());
        }
        TokenCase::VertexEmbedding => {
            let input = TokenUrlInput {
                model: "text-embedding-005",
                mode: UrlMode::Embedding,
                location: Some("global"),
                ..token_input!(Some(&auth))
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
                api_version: ApiVersion::V1Beta1,
                ..token_input!(Some(&auth))
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
                ..token_input!(Some(&auth))
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
                ..token_input!(Some(&auth))
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
                stream: true,
                gemini_api_key: Some(&key),
                project: None,
                location: None,
                ..token_input!(None)
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
                ..token_input!(None)
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
                ..token_input!(Some(&auth))
            };
            let target = get_token_and_url(&catalog(), &input).unwrap();
            assert!(matches!(target.auth, Some(AuthHeader::Headers(_))));
            assert_eq!(header_of(&target, "x-goog-api-key"), Some("gk"));
        }
    }
}

#[test]
fn token_and_url_gemini_ignores_psc_format() {
    let key = secret("gk");
    let input = TokenUrlInput {
        provider: TargetProvider::Gemini,
        gemini_api_key: Some(&key),
        api_base: Some("https://custom.example.com"),
        use_psc_endpoint_format: true,
        ..token_input!(None)
    };
    assert_eq!(
        get_token_and_url(&catalog(), &input).unwrap().url,
        "https://custom.example.com/models/gemini-2.5-flash:generateContent"
    );
}

#[test]
fn token_and_url_requires_a_gemini_key() {
    let input = TokenUrlInput {
        provider: TargetProvider::Gemini,
        ..token_input!(None)
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
        ..token_input!(None)
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
        ..token_input!(None)
    };
    assert!(matches!(
        get_token_and_url(&catalog(), &input).unwrap_err(),
        EndpointTargetError::VertexUrl(VertexUrlError::UnsupportedMode { .. })
    ));
}
