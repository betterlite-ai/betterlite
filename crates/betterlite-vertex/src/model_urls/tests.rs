#![cfg(test)]

use std::sync::Arc;

use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use rstest::rstest;

use crate::model_urls::base_url::{get_vertex_base_url, validate_vertex_location};
use crate::model_urls::embedding::{EmbeddingUrlInput, get_embedding_url};
use crate::model_urls::error::VertexUrlError;
use crate::model_urls::gemini::{GeminiUrlInput, get_gemini_url};
use crate::model_urls::model_name::{
    get_fine_tuned_endpoint_id, get_model_for_vertex_ai_url, get_vertex_base_model_name,
    is_gemini_3_or_newer,
};
use crate::model_urls::vertex::{VertexUrlInput, get_vertex_url};
use crate::model_urls::{ApiVersion, UrlMode, VertexUrl};

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

fn chat_url(
    catalog: &ModelCatalog,
    model: &str,
    stream: Option<bool>,
    location: &str,
    api_version: ApiVersion,
) -> Result<VertexUrl, VertexUrlError> {
    get_vertex_url(
        catalog,
        &VertexUrlInput {
            mode: UrlMode::Chat,
            model,
            stream,
            project: Some("proj"),
            location: Some(location),
            api_version,
        },
    )
}

fn image_url(
    catalog: &ModelCatalog,
    model: &str,
    location: &str,
    api_version: ApiVersion,
) -> Result<VertexUrl, VertexUrlError> {
    get_vertex_url(
        catalog,
        &VertexUrlInput {
            mode: UrlMode::ImageGeneration,
            model,
            stream: None,
            project: Some("proj"),
            location: Some(location),
            api_version,
        },
    )
}

fn count_url(
    catalog: &ModelCatalog,
    model: &str,
    location: &str,
    api_version: ApiVersion,
) -> Result<VertexUrl, VertexUrlError> {
    get_vertex_url(
        catalog,
        &VertexUrlInput {
            mode: UrlMode::CountTokens,
            model,
            stream: None,
            project: Some("proj"),
            location: Some(location),
            api_version,
        },
    )
}

fn vertex_embedding_url(
    catalog: &ModelCatalog,
    model: &str,
    location: &str,
    api_version: ApiVersion,
) -> Result<VertexUrl, VertexUrlError> {
    get_vertex_url(
        catalog,
        &VertexUrlInput {
            mode: UrlMode::Embedding,
            model,
            stream: None,
            project: Some("proj"),
            location: Some(location),
            api_version,
        },
    )
}

fn embedding_url(
    catalog: &ModelCatalog,
    model: &str,
    location: &str,
    api_version: ApiVersion,
) -> Result<VertexUrl, VertexUrlError> {
    get_embedding_url(
        catalog,
        &EmbeddingUrlInput {
            model,
            project: Some("proj"),
            location: Some(location),
            api_version,
        },
    )
}

fn gemini_url(
    model: &str,
    mode: UrlMode,
    stream: Option<bool>,
) -> Result<VertexUrl, VertexUrlError> {
    get_gemini_url(&GeminiUrlInput {
        mode,
        model,
        stream,
    })
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
    let global = chat_url(&catalog, "gemini-2.5-flash", None, "global", ApiVersion::V1).unwrap();
    assert_eq!(
        global.url,
        "https://aiplatform.googleapis.com/v1/projects/proj/locations/global/publishers/google/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(global.endpoint, "generateContent");
    streaming_chat();
}

fn streaming_chat() {
    let catalog = catalog();
    let regional = chat_url(
        &catalog,
        "gemini-2.5-flash",
        Some(true),
        "us-central1",
        ApiVersion::V1Beta1,
    )
    .unwrap();
    assert_eq!(
        regional.url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
    );
    assert_eq!(regional.endpoint, "streamGenerateContent");
    let not_streaming = chat_url(
        &catalog,
        "gemini-2.5-flash",
        Some(false),
        "us-central1",
        ApiVersion::V1Beta1,
    )
    .unwrap();
    assert_eq!(not_streaming.endpoint, "generateContent");
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
    let catalog = catalog();
    let count = count_url(&catalog, "gemini-2.5-flash", "eu", ApiVersion::V1Beta1).unwrap();
    assert_eq!(
        count.url,
        "https://aiplatform.eu.rep.googleapis.com/v1beta1/projects/proj/locations/eu/publishers/google/models/gemini-2.5-flash:countTokens"
    );
    assert_eq!(count.endpoint, "countTokens");
    let image = image_url(&catalog, "imagen-3.0-generate-001", "us", ApiVersion::V1).unwrap();
    assert_eq!(
        image.url,
        "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/publishers/google/models/imagen-3.0-generate-001:predict"
    );
    let embedding = embedding_url(&catalog, "text-embedding-005", "us", ApiVersion::V1).unwrap();
    assert_eq!(
        embedding.url,
        "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/publishers/google/models/text-embedding-005:predict"
    );
}

fn numeric_embedding() {
    let catalog = catalog();
    let bge = embedding_url(&catalog, "bge/123456", "us-central1", ApiVersion::V1Beta1).unwrap();
    assert_eq!(
        bge.url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/endpoints/123456:predict"
    );
    assert_eq!(bge.endpoint, "predict");
    let digits = embedding_url(
        &catalog,
        "4965075652664360960",
        "global",
        ApiVersion::V1Beta1,
    )
    .unwrap();
    assert_eq!(
        digits.url,
        "https://aiplatform.googleapis.com/v1beta1/projects/proj/locations/global/endpoints/4965075652664360960:predict"
    );
    let spec = vertex_embedding_url(&catalog, "gemini/123456", "global", ApiVersion::V1).unwrap();
    assert_eq!(
        spec.url,
        "https://aiplatform.googleapis.com/v1/projects/proj/locations/global/endpoints/123456:predict"
    );
    let publisher = embedding_url(
        &catalog,
        "text-embedding-005",
        "us-central1",
        ApiVersion::V1Beta1,
    )
    .unwrap();
    assert_eq!(
        publisher.url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/text-embedding-005:predict"
    );
}

fn embed_content_catalog() {
    let catalog = catalog();
    let plain = embedding_url(
        &catalog,
        "gemini-embedding-2",
        "us-central1",
        ApiVersion::V1,
    )
    .unwrap();
    assert_eq!(
        plain.url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-embedding-2:embedContent"
    );
    assert_eq!(plain.endpoint, "embedContent");
    let preview =
        embedding_url(&catalog, "gemini-embedding-2-preview", "us", ApiVersion::V1).unwrap();
    assert_eq!(
        preview.url,
        "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/publishers/google/models/gemini-embedding-2-preview:embedContent"
    );
    let predict = embedding_url(
        &catalog,
        "text-embedding-005",
        "us-central1",
        ApiVersion::V1,
    )
    .unwrap();
    assert_eq!(predict.endpoint, "predict");
    prefixed_embed_content();
}

fn prefixed_embed_content() {
    let catalog = catalog();
    let prefixed = embedding_url(
        &catalog,
        "vertex_ai/gemini-embedding-2",
        "europe-west1",
        ApiVersion::V1Beta1,
    )
    .unwrap();
    assert_eq!(
        prefixed.url,
        "https://europe-west1-aiplatform.googleapis.com/v1/projects/proj/locations/europe-west1/publishers/google/models/vertex_ai/gemini-embedding-2:embedContent"
    );
    let spec = vertex_embedding_url(
        &catalog,
        "gemini/gemini-embedding-2",
        "us-central1",
        ApiVersion::V1,
    )
    .unwrap();
    assert_eq!(
        spec.url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-embedding-2:embedContent"
    );
}

fn gemini_version_aliases() {
    assert!(!is_gemini_3_or_newer("gemini-2.5-flash"));
    assert!(!is_gemini_3_or_newer("gemini-flash"));
    assert!(is_gemini_3_or_newer("gemini-flash-latest"));
    assert!(!is_gemini_3_or_newer("gemini-pro"));
    assert!(is_gemini_3_or_newer("gemini-pro-latest"));
    assert!(is_gemini_3_or_newer("gemini-3-pro-preview"));
    assert!(!is_gemini_3_or_newer("gemini/1234"));
    assert!(!is_gemini_3_or_newer("gemini/ft-abc"));
    assert!(!is_gemini_3_or_newer("1234"));
    assert!(!is_gemini_3_or_newer("gemma-3-12b-it"));
    assert!(is_gemini_3_or_newer("gemini-10-pro"));
    assert!(!is_gemini_3_or_newer(""));
    gemini_version_urls();
}

fn gemini_version_urls() {
    let old = gemini_url("gemini-2.5-flash", UrlMode::Chat, None).unwrap();
    assert_eq!(
        old.url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(old.endpoint, "generateContent");
    let new = gemini_url("gemini-3-pro-preview", UrlMode::Chat, Some(true)).unwrap();
    assert_eq!(
        new.url,
        "https://generativelanguage.googleapis.com/v1alpha/models/gemini-3-pro-preview:streamGenerateContent?alt=sse"
    );
    assert_eq!(new.endpoint, "streamGenerateContent");
    let latest = gemini_url("gemini-flash-latest", UrlMode::Chat, None).unwrap();
    assert_eq!(
        latest.url,
        "https://generativelanguage.googleapis.com/v1alpha/models/gemini-flash-latest:generateContent"
    );
    let pinned = gemini_url("gemini-flash", UrlMode::Chat, None).unwrap();
    assert_eq!(
        pinned.url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-flash:generateContent"
    );
    let embedding = gemini_url("gemini-3-pro-preview", UrlMode::Embedding, None).unwrap();
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
    let spec = chat_url(
        &catalog,
        "vertex_ai/gemini/987654",
        None,
        "us",
        ApiVersion::V1,
    )
    .unwrap();
    assert_eq!(
        spec.url,
        "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/endpoints/987654:generateContent"
    );
    let digits = chat_url(
        &catalog,
        "4965075652664360960",
        Some(true),
        "us-central1",
        ApiVersion::V1Beta1,
    )
    .unwrap();
    assert_eq!(
        digits.url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/endpoints/4965075652664360960:streamGenerateContent?alt=sse"
    );
    assert_eq!(digits.endpoint, "streamGenerateContent");
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
    assert!(matches!(
        validate_vertex_location(Some("attacker.example/")),
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
        &VertexUrlInput {
            mode: UrlMode::BatchEmbedding,
            model: "gemini-embedding-2",
            stream: None,
            project: Some("proj"),
            location: Some("us-central1"),
            api_version: ApiVersion::V1,
        },
    );
    assert!(matches!(
        batch,
        Err(VertexUrlError::UnsupportedMode {
            mode: "batch_embedding"
        })
    ));
    // the rejected mode never validates the location first
    let unvalidated = get_vertex_url(
        &catalog,
        &VertexUrlInput {
            mode: UrlMode::BatchEmbedding,
            model: "gemini-embedding-2",
            stream: None,
            project: Some("proj"),
            location: Some("bad location/"),
            api_version: ApiVersion::V1,
        },
    );
    assert!(matches!(
        unvalidated,
        Err(VertexUrlError::UnsupportedMode { .. })
    ));
}

fn gemini_unsupported_modes() {
    assert!(matches!(
        gemini_url("gemini-2.5-flash", UrlMode::ImageGeneration, None),
        Err(VertexUrlError::GeminiImageGeneration)
    ));
    let batch_gemini = gemini_url("gemini-2.5-flash", UrlMode::BatchEmbedding, None).unwrap();
    assert_eq!(
        batch_gemini.url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:batchEmbedContents"
    );
    assert_eq!(batch_gemini.endpoint, "batchEmbedContents");
}
