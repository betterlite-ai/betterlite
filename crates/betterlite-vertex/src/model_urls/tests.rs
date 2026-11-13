#![cfg(test)]

use std::sync::Arc;

use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use rstest::rstest;

use crate::model_urls::base_url::{get_vertex_base_url, validate_vertex_location};
use crate::model_urls::embedding::{EmbeddingUrlInput, get_embedding_url};
use crate::model_urls::error::VertexUrlError;
use crate::model_urls::gemini::{GeminiUrlInput, get_gemini_url};
use crate::model_urls::model_name::{
    forward_gemini_call_id, get_fine_tuned_endpoint_id, get_model_for_vertex_ai_url,
    get_vertex_base_model_name, is_gemini_3_or_newer,
};
use crate::model_urls::vertex::{VertexUrlInput, get_vertex_url};
use crate::model_urls::{ApiVersion, Endpoint, UrlMode};

const PROJECT: &str = "proj";

#[derive(Debug, Clone, Copy)]
enum Scenario {
    RegionalAndGlobal,
    GeographyHost,
    NumericEmbedding,
    EmbedContentCatalog,
    GeminiVersionAliases,
    FineTunedGemini,
    InvalidLocation,
    UnsupportedMode,
}

#[rstest]
#[case::regional_and_global(Scenario::RegionalAndGlobal)]
#[case::geography_host(Scenario::GeographyHost)]
#[case::numeric_embedding(Scenario::NumericEmbedding)]
#[case::embed_content_catalog(Scenario::EmbedContentCatalog)]
#[case::gemini_version_aliases(Scenario::GeminiVersionAliases)]
#[case::fine_tuned_gemini(Scenario::FineTunedGemini)]
#[case::invalid_location(Scenario::InvalidLocation)]
#[case::unsupported_mode(Scenario::UnsupportedMode)]
fn endpoint_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::RegionalAndGlobal => regional_and_global(),
        Scenario::GeographyHost => geography_host(),
        Scenario::NumericEmbedding => numeric_embedding(),
        Scenario::EmbedContentCatalog => embed_content_catalog(),
        Scenario::GeminiVersionAliases => gemini_version_aliases(),
        Scenario::FineTunedGemini => fine_tuned_gemini(),
        Scenario::InvalidLocation => invalid_location(),
        Scenario::UnsupportedMode => unsupported_mode(),
    }
}

fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

fn vertex_input<'text>(
    mode: UrlMode,
    model: &'text str,
    stream: Option<bool>,
    location: &'text str,
    version: ApiVersion,
) -> VertexUrlInput<'text> {
    VertexUrlInput {
        mode,
        model,
        stream,
        project: Some(PROJECT),
        location: Some(location),
        api_version: version,
    }
}

fn embedding_input<'text>(
    model: &'text str,
    location: &'text str,
    version: ApiVersion,
) -> EmbeddingUrlInput<'text> {
    EmbeddingUrlInput {
        model,
        project: Some(PROJECT),
        location: Some(location),
        api_version: version,
    }
}

fn gemini_input(mode: UrlMode, model: &str, stream: Option<bool>) -> GeminiUrlInput<'_> {
    GeminiUrlInput {
        mode,
        model,
        stream,
    }
}

fn regional_and_global() {
    assert_eq!(
        get_vertex_base_url(Some("global")).unwrap(),
        "https://aiplatform.googleapis.com"
    );
    assert_eq!(
        get_vertex_base_url(Some("us-central1")).unwrap(),
        "https://us-central1-aiplatform.googleapis.com"
    );
    assert_eq!(
        get_vertex_base_url(Some("europe-west1")).unwrap(),
        "https://europe-west1-aiplatform.googleapis.com"
    );
    let catalog = catalog();
    let global = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::Chat,
            "gemini-2.5-flash",
            None,
            "global",
            ApiVersion::V1,
        ),
    )
    .unwrap();
    assert_eq!(
        global.url,
        "https://aiplatform.googleapis.com/v1/projects/proj/locations/global/publishers/google/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(global.endpoint, Endpoint::GenerateContent);
    streaming_chat();
}

fn streaming_chat() {
    let catalog = catalog();
    let regional = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::Chat,
            "gemini-2.5-flash",
            Some(true),
            "us-central1",
            ApiVersion::V1Beta1,
        ),
    )
    .unwrap();
    assert_eq!(
        regional.url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
    );
    assert_eq!(regional.endpoint, Endpoint::StreamGenerateContent);
    let not_streaming = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::Chat,
            "gemini-2.5-flash",
            Some(false),
            "us-central1",
            ApiVersion::V1Beta1,
        ),
    )
    .unwrap();
    assert_eq!(not_streaming.endpoint, Endpoint::GenerateContent);
    assert!(!not_streaming.url.contains("alt=sse"));
}

fn geography_host() {
    assert_eq!(
        get_vertex_base_url(Some("us")).unwrap(),
        "https://aiplatform.us.rep.googleapis.com"
    );
    assert_eq!(
        get_vertex_base_url(Some("eu")).unwrap(),
        "https://aiplatform.eu.rep.googleapis.com"
    );
    geography_vertex_urls();
    geography_embedding_url();
}

fn geography_vertex_urls() {
    let catalog = catalog();
    let count = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::CountTokens,
            "gemini-2.5-flash",
            None,
            "eu",
            ApiVersion::V1Beta1,
        ),
    )
    .unwrap();
    assert_eq!(
        count.url,
        "https://aiplatform.eu.rep.googleapis.com/v1beta1/projects/proj/locations/eu/publishers/google/models/gemini-2.5-flash:countTokens"
    );
    assert_eq!(count.endpoint, Endpoint::CountTokens);
    let image = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::ImageGeneration,
            "imagen-3.0-generate-001",
            None,
            "us",
            ApiVersion::V1,
        ),
    )
    .unwrap();
    assert_eq!(
        image.url,
        "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/publishers/google/models/imagen-3.0-generate-001:predict"
    );
}

fn geography_embedding_url() {
    let catalog = catalog();
    let embedding = get_embedding_url(
        &catalog,
        &embedding_input("text-embedding-005", "us", ApiVersion::V1),
    )
    .unwrap();
    assert_eq!(
        embedding.url,
        "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/publishers/google/models/text-embedding-005:predict"
    );
}

fn numeric_embedding() {
    numeric_endpoint_embedding();
    publisher_embedding_version();
}

fn numeric_endpoint_embedding() {
    let catalog = catalog();
    let bge = get_embedding_url(
        &catalog,
        &embedding_input("bge/123456", "us-central1", ApiVersion::V1Beta1),
    )
    .unwrap();
    assert_eq!(
        bge.url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/endpoints/123456:predict"
    );
    assert_eq!(bge.endpoint, Endpoint::Predict);
    let digits = get_embedding_url(
        &catalog,
        &embedding_input("4965075652664360960", "global", ApiVersion::V1Beta1),
    )
    .unwrap();
    assert_eq!(
        digits.url,
        "https://aiplatform.googleapis.com/v1beta1/projects/proj/locations/global/endpoints/4965075652664360960:predict"
    );
    let spec = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::Embedding,
            "gemini/123456",
            None,
            "global",
            ApiVersion::V1,
        ),
    )
    .unwrap();
    assert_eq!(
        spec.url,
        "https://aiplatform.googleapis.com/v1/projects/proj/locations/global/endpoints/123456:predict"
    );
}

fn publisher_embedding_version() {
    let catalog = catalog();
    let publisher = get_embedding_url(
        &catalog,
        &embedding_input("text-embedding-005", "us-central1", ApiVersion::V1Beta1),
    )
    .unwrap();
    assert_eq!(
        publisher.url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/text-embedding-005:predict"
    );
}

fn catalog_embedding(
    catalog: &ModelCatalog,
    model: &str,
    location: &str,
    version: ApiVersion,
) -> (String, Endpoint) {
    let url = get_embedding_url(catalog, &embedding_input(model, location, version)).unwrap();
    (url.url, url.endpoint)
}

fn embed_content_catalog() {
    let catalog = catalog();
    let plain = get_embedding_url(
        &catalog,
        &embedding_input("gemini-embedding-2", "us-central1", ApiVersion::V1),
    )
    .unwrap();
    assert_eq!(
        plain.url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-embedding-2:embedContent"
    );
    assert_eq!(plain.endpoint, Endpoint::EmbedContent);
    let preview = get_embedding_url(
        &catalog,
        &embedding_input("gemini-embedding-2-preview", "us", ApiVersion::V1),
    )
    .unwrap();
    assert_eq!(
        preview.url,
        "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/publishers/google/models/gemini-embedding-2-preview:embedContent"
    );
    let predict = get_embedding_url(
        &catalog,
        &embedding_input("text-embedding-005", "us-central1", ApiVersion::V1),
    )
    .unwrap();
    assert_eq!(predict.endpoint, Endpoint::Predict);
    model_name_variants();
    spec_model_embedding();
}

fn model_name_variants() {
    let catalog = catalog();
    assert_eq!(
        catalog_embedding(&catalog, "vertex_ai/gemini-embedding-2", "europe-west1", ApiVersion::V1Beta1),
        (
            "https://europe-west1-aiplatform.googleapis.com/v1/projects/proj/locations/europe-west1/publishers/google/models/vertex_ai/gemini-embedding-2:embedContent".to_owned(),
            Endpoint::EmbedContent,
        )
    );
    assert_eq!(
        catalog_embedding(&catalog, "vertex_ai/gemini-embedding-2-001", "us-central1", ApiVersion::V1),
        (
            "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/vertex_ai/gemini-embedding-2-001:embedContent".to_owned(),
            Endpoint::EmbedContent,
        )
    );
    assert_eq!(
        catalog_embedding(&catalog, "vertex_ai/vertex_ai/gemini-embedding-2", "us-central1", ApiVersion::V1),
        (
            "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/vertex_ai/vertex_ai/gemini-embedding-2:embedContent".to_owned(),
            Endpoint::EmbedContent,
        )
    );
    assert_eq!(
        catalog_embedding(&catalog, "gemini-flash-experimental-001", "us-central1", ApiVersion::V1),
        (
            "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-flash-experimental-001:embedContent".to_owned(),
            Endpoint::EmbedContent,
        )
    );
}

fn spec_model_embedding() {
    let catalog = catalog();
    let spec = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::Embedding,
            "gemini/gemini-embedding-2",
            None,
            "us-central1",
            ApiVersion::V1,
        ),
    )
    .unwrap();
    assert_eq!(
        spec.url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-embedding-2:embedContent"
    );
    assert_eq!(spec.endpoint, Endpoint::EmbedContent);
    assert_eq!(
        catalog_embedding(
            &catalog,
            "gemini-embedding-2-001",
            "us-central1",
            ApiVersion::V1
        )
        .1,
        Endpoint::EmbedContent
    );
}

fn gemini_version_aliases() {
    older_gemini_names();
    newer_gemini_names();
    assert!(forward_gemini_call_id("gemini-3-pro-preview"));
    assert!(!forward_gemini_call_id("gemini-2.5-flash"));
    gemini_version_urls();
}

fn older_gemini_names() {
    assert!(!is_gemini_3_or_newer("gemini-2.5-flash"));
    assert!(!is_gemini_3_or_newer("gemini-flash"));
    assert!(!is_gemini_3_or_newer("gemini-1.5-pro"));
    assert!(!is_gemini_3_or_newer("gemini-1"));
    assert!(!is_gemini_3_or_newer("gemini-exp-1206"));
    assert!(!is_gemini_3_or_newer("gemini-flash-lite"));
    assert!(!is_gemini_3_or_newer("gemini-pro"));
    assert!(!is_gemini_3_or_newer("gemini/1234"));
    assert!(!is_gemini_3_or_newer("gemini/ft-abc"));
    assert!(!is_gemini_3_or_newer("1234"));
    assert!(!is_gemini_3_or_newer("gemma-3-12b-it"));
    assert!(!is_gemini_3_or_newer(""));
}

fn newer_gemini_names() {
    assert!(is_gemini_3_or_newer("gemini-flash-latest"));
    assert!(is_gemini_3_or_newer("gemini-pro-latest"));
    assert!(is_gemini_3_or_newer("gemini-3-pro-preview"));
    assert!(is_gemini_3_or_newer("gemini-10-pro"));
    assert!(is_gemini_3_or_newer("gemini-flash-lite-latest"));
    assert!(is_gemini_3_or_newer("gemini-pro-lite-latest"));
    assert!(is_gemini_3_or_newer("gemini-experimental"));
}

fn gemini_version_urls() {
    let old = get_gemini_url(&gemini_input(UrlMode::Chat, "gemini-2.5-flash", None)).unwrap();
    assert_eq!(
        old.url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(old.endpoint, Endpoint::GenerateContent);
    let new = get_gemini_url(&gemini_input(
        UrlMode::Chat,
        "gemini-3-pro-preview",
        Some(true),
    ))
    .unwrap();
    assert_eq!(
        new.url,
        "https://generativelanguage.googleapis.com/v1alpha/models/gemini-3-pro-preview:streamGenerateContent?alt=sse"
    );
    assert_eq!(new.endpoint, Endpoint::StreamGenerateContent);
    let latest = get_gemini_url(&gemini_input(UrlMode::Chat, "gemini-flash-latest", None)).unwrap();
    assert_eq!(
        latest.url,
        "https://generativelanguage.googleapis.com/v1alpha/models/gemini-flash-latest:generateContent"
    );
    let pinned = get_gemini_url(&gemini_input(UrlMode::Chat, "gemini-flash", None)).unwrap();
    assert_eq!(
        pinned.url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-flash:generateContent"
    );
    let embedding = get_gemini_url(&gemini_input(
        UrlMode::Embedding,
        "gemini-3-pro-preview",
        None,
    ))
    .unwrap();
    assert_eq!(
        embedding.url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-3-pro-preview:embedContent"
    );
}

fn fine_tuned_gemini() {
    assert_eq!(get_fine_tuned_endpoint_id("123"), Some("123"));
    assert_eq!(get_fine_tuned_endpoint_id("vertex_ai/123"), None);
    assert_eq!(
        get_fine_tuned_endpoint_id("vertex_ai/gemini/456"),
        Some("456")
    );
    assert_eq!(get_fine_tuned_endpoint_id("gemini/789"), Some("789"));
    assert_eq!(get_fine_tuned_endpoint_id("gemini-2.5-flash"), None);
    assert_eq!(get_fine_tuned_endpoint_id(""), None);
    assert_eq!(get_model_for_vertex_ai_url("vertex_ai/gemini/123"), "123");
    assert_eq!(get_model_for_vertex_ai_url("gemini/a/b"), "b");
    assert_eq!(get_model_for_vertex_ai_url("gemini-3-pro"), "gemini-3-pro");
    assert_eq!(get_model_for_vertex_ai_url("GEMINI/x"), "GEMINI/x");
    fine_tuned_model_names();
    fine_tuned_urls();
}

fn fine_tuned_model_names() {
    assert_eq!(get_vertex_base_model_name("bge/123"), "123");
    assert_eq!(
        get_vertex_base_model_name("gemma/gemma-3-12b-it"),
        "gemma-3-12b-it"
    );
    assert_eq!(get_vertex_base_model_name("partner_models/x"), "x");
    assert_eq!(get_vertex_base_model_name("model_garden/y"), "y");
    assert_eq!(get_vertex_base_model_name("non_gemini/z"), "z");
    assert_eq!(get_vertex_base_model_name("openai/w"), "w");
    assert_eq!(get_vertex_base_model_name("agent_engine/v"), "v");
    assert_eq!(get_vertex_base_model_name("bge/bge/x"), "bge/x");
    assert_eq!(
        get_vertex_base_model_name("vertex_ai/gemini/456"),
        "vertex_ai/gemini/456"
    );
}

fn fine_tuned_urls() {
    let catalog = catalog();
    let spec = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::Chat,
            "vertex_ai/gemini/987654",
            None,
            "us",
            ApiVersion::V1,
        ),
    )
    .unwrap();
    assert_eq!(
        spec.url,
        "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/endpoints/987654:generateContent"
    );
    let digits = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::Chat,
            "4965075652664360960",
            Some(true),
            "us-central1",
            ApiVersion::V1Beta1,
        ),
    )
    .unwrap();
    assert_eq!(
        digits.url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/endpoints/4965075652664360960:streamGenerateContent?alt=sse"
    );
    assert_eq!(digits.endpoint, Endpoint::StreamGenerateContent);
}

fn invalid_location() {
    missing_location();
    malformed_locations();
    accepted_locations();
}

fn missing_location() {
    assert!(matches!(
        validate_vertex_location(None),
        Err(VertexUrlError::LocationRequired)
    ));
    assert!(matches!(
        get_vertex_base_url(None),
        Err(VertexUrlError::LocationRequired)
    ));
}

fn malformed_locations() {
    malformed_hosts();
    malformed_first_characters();
}

fn malformed_hosts() {
    assert!(matches!(
        validate_vertex_location(Some("attacker.example/")),
        Err(VertexUrlError::InvalidLocation)
    ));
    // Python's $ anchor accepts a trailing newline; the location must not carry one into the host
    assert!(matches!(
        validate_vertex_location(Some("us\n")),
        Err(VertexUrlError::InvalidLocation)
    ));
    assert!(matches!(
        validate_vertex_location(Some("evil.com#")),
        Err(VertexUrlError::InvalidLocation)
    ));
    assert!(matches!(
        validate_vertex_location(Some("US")),
        Err(VertexUrlError::InvalidLocation)
    ));
    assert!(matches!(
        validate_vertex_location(Some("us_central1")),
        Err(VertexUrlError::InvalidLocation)
    ));
    assert!(matches!(
        validate_vertex_location(Some("")),
        Err(VertexUrlError::InvalidLocation)
    ));
    assert!(matches!(
        get_vertex_base_url(Some("evil.com#")),
        Err(VertexUrlError::InvalidLocation)
    ));
}

// the first character must be a lowercase letter, so a digit or hyphen cannot start a host
fn malformed_first_characters() {
    assert!(matches!(
        validate_vertex_location(Some("1us")),
        Err(VertexUrlError::InvalidLocation)
    ));
    assert!(matches!(
        validate_vertex_location(Some("-us")),
        Err(VertexUrlError::InvalidLocation)
    ));
}

fn accepted_locations() {
    assert_eq!(validate_vertex_location(Some("global")).unwrap(), "global");
    assert_eq!(validate_vertex_location(Some("us")).unwrap(), "us");
    assert_eq!(
        validate_vertex_location(Some("us-central1")).unwrap(),
        "us-central1"
    );
}

fn unsupported_mode() {
    native_batch_embedding();
    gemini_unsupported_modes();
}

fn native_batch_embedding() {
    let catalog = catalog();
    let batch = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::BatchEmbedding,
            "gemini-embedding-2",
            None,
            "us-central1",
            ApiVersion::V1,
        ),
    );
    assert!(matches!(
        batch,
        Err(VertexUrlError::UnsupportedMode {
            mode: UrlMode::BatchEmbedding
        })
    ));
    // the rejected mode never validates the location first
    let unvalidated = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::BatchEmbedding,
            "gemini-embedding-2",
            None,
            "bad location/",
            ApiVersion::V1,
        ),
    );
    assert!(matches!(
        unvalidated,
        Err(VertexUrlError::UnsupportedMode { .. })
    ));
}

fn gemini_unsupported_modes() {
    assert!(matches!(
        get_gemini_url(&gemini_input(
            UrlMode::ImageGeneration,
            "gemini-2.5-flash",
            None
        )),
        Err(VertexUrlError::GeminiImageGeneration)
    ));
    let batch_gemini = get_gemini_url(&gemini_input(
        UrlMode::BatchEmbedding,
        "gemini-2.5-flash",
        None,
    ))
    .unwrap();
    assert_eq!(
        batch_gemini.url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:batchEmbedContents"
    );
    assert_eq!(batch_gemini.endpoint, Endpoint::BatchEmbedContents);
}

#[test]
fn image_generation_keeps_its_version_rules() {
    let catalog = catalog();
    let numeric = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::ImageGeneration,
            "1234567890",
            None,
            "us-central1",
            ApiVersion::V1Beta1,
        ),
    )
    .unwrap();
    assert_eq!(
        numeric.url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/endpoints/1234567890:predict"
    );
    assert_eq!(numeric.endpoint, Endpoint::Predict);
    let publisher = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::ImageGeneration,
            "imagen-3.0-generate-001",
            None,
            "us",
            ApiVersion::V1Beta1,
        ),
    )
    .unwrap();
    assert_eq!(
        publisher.url,
        "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/publishers/google/models/imagen-3.0-generate-001:predict"
    );
}

#[test]
fn count_tokens_always_uses_the_publisher_path() {
    let catalog = catalog();
    let count = get_vertex_url(
        &catalog,
        &vertex_input(
            UrlMode::CountTokens,
            "gemini/1234",
            None,
            "us",
            ApiVersion::V1,
        ),
    )
    .unwrap();
    assert_eq!(
        count.url,
        "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/publishers/google/models/1234:countTokens"
    );
    assert_eq!(count.endpoint, Endpoint::CountTokens);
}

#[test]
fn non_ascii_digits_are_not_endpoint_ids() {
    assert_eq!(get_fine_tuned_endpoint_id("\u{661}\u{662}\u{663}"), None);
    assert!(is_gemini_3_or_newer("\u{661}\u{662}\u{663}"));
    assert!(is_gemini_3_or_newer("gemini-2.\u{665}-pro"));
}

#[test]
fn trailing_newline_is_not_an_old_gemini_name() {
    assert!(is_gemini_3_or_newer("gemini-2\n"));
    assert!(is_gemini_3_or_newer("gemini-pro\n"));
}

#[test]
fn missing_project_is_an_error() {
    let catalog = catalog();
    let embedding = get_embedding_url(
        &catalog,
        &EmbeddingUrlInput {
            model: "text-embedding-005",
            project: None,
            location: Some("us-central1"),
            api_version: ApiVersion::V1,
        },
    );
    assert!(matches!(embedding, Err(VertexUrlError::ProjectRequired)));
    let chat = get_vertex_url(
        &catalog,
        &VertexUrlInput {
            mode: UrlMode::Chat,
            model: "gemini-2.5-flash",
            stream: None,
            project: None,
            location: Some("us-central1"),
            api_version: ApiVersion::V1,
        },
    );
    assert!(matches!(chat, Err(VertexUrlError::ProjectRequired)));
}

#[test]
fn catalog_lookup_is_case_sensitive() {
    let catalog = catalog();
    let other_case = get_embedding_url(
        &catalog,
        &embedding_input("Gemini-Embedding-2", "us-central1", ApiVersion::V1),
    )
    .unwrap();
    assert_eq!(other_case.endpoint, Endpoint::Predict);
}
