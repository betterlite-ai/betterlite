use betterlite_models::catalog::BundledCatalog;
use betterlite_vertex::model_urls::base_url::get_vertex_base_url;
use betterlite_vertex::model_urls::embedding::{EmbeddingUrlInput, get_embedding_url};
use betterlite_vertex::model_urls::error::VertexUrlError;
use betterlite_vertex::model_urls::gemini::{GeminiUrlInput, get_gemini_url};
use betterlite_vertex::model_urls::model_name::{
    get_fine_tuned_endpoint_id, get_model_for_vertex_ai_url, get_vertex_base_model_name,
    is_gemini_3_or_newer,
};
use betterlite_vertex::model_urls::vertex::{VertexUrlInput, get_vertex_url};
use betterlite_vertex::model_urls::{ApiVersion, UrlMode, VertexUrl};
use serde_json::{Value, json};

use super::input::{EmbeddingUrlCase, GeminiUrlCase, Input, UnsupportedMode, VertexUrlCase};

pub(super) fn run(input: &Input) -> Value {
    match input {
        Input::BaseUrl { locations } => base_urls(locations),
        Input::VertexUrl { cases } => vertex_urls(cases),
        Input::EmbeddingUrl { cases } => embedding_urls(cases),
        Input::GeminiUrl { cases } => gemini_urls(cases),
        Input::ModelPredicates { models } => model_predicates(models),
        Input::UnsupportedModes { modes } => unsupported_modes(modes),
    }
}

fn url_value(result: Result<VertexUrl, VertexUrlError>) -> Value {
    result.map_or_else(
        |_error| json!({"error": true}),
        |url| json!({"url": url.url, "endpoint": url.endpoint}),
    )
}

fn base_urls(locations: &[Option<String>]) -> Value {
    let urls: Vec<Value> = locations
        .iter()
        .map(|location| {
            get_vertex_base_url(location.as_deref())
                .map_or_else(|_error| json!({"error": true}), |url| json!({"url": url}))
        })
        .collect();
    json!({"urls": urls})
}

fn vertex_urls(cases: &[VertexUrlCase]) -> Value {
    let catalog = BundledCatalog::default().get();
    let urls: Vec<Value> = cases
        .iter()
        .map(|case| {
            url_value(get_vertex_url(
                &catalog,
                &VertexUrlInput {
                    mode: case.mode.into_mode(),
                    model: &case.model,
                    stream: case.stream,
                    project: case.project.as_deref(),
                    location: case.location.as_deref(),
                    api_version: case.api_version.into_version(),
                },
            ))
        })
        .collect();
    json!({"urls": urls})
}

fn embedding_urls(cases: &[EmbeddingUrlCase]) -> Value {
    let catalog = BundledCatalog::default().get();
    let urls: Vec<Value> = cases
        .iter()
        .map(|case| {
            url_value(get_embedding_url(
                &catalog,
                &EmbeddingUrlInput {
                    model: &case.model,
                    project: case.project.as_deref(),
                    location: case.location.as_deref(),
                    api_version: case.api_version.into_version(),
                },
            ))
        })
        .collect();
    json!({"urls": urls})
}

fn gemini_urls(cases: &[GeminiUrlCase]) -> Value {
    let urls: Vec<Value> = cases
        .iter()
        .map(|case| {
            url_value(get_gemini_url(&GeminiUrlInput {
                mode: case.mode.into_mode(),
                model: &case.model,
                stream: case.stream,
            }))
        })
        .collect();
    json!({"urls": urls})
}

fn model_predicates(models: &[String]) -> Value {
    let entries: Vec<Value> = models
        .iter()
        .map(|model| {
            let newer = is_gemini_3_or_newer(model);
            json!({
                "base_name": get_vertex_base_model_name(model),
                "fine_tuned_id": get_fine_tuned_endpoint_id(model),
                "url_model": get_model_for_vertex_ai_url(model),
                "newer": newer,
                "forward": newer,
            })
        })
        .collect();
    json!({"models": entries})
}

fn unsupported_modes(modes: &[UnsupportedMode]) -> Value {
    let results: Vec<Value> = modes
        .iter()
        .map(|mode| match mode {
            UnsupportedMode::BatchEmbedding => json!({
                "mode": "batch_embedding",
                "error": batch_embedding_rejected(),
            }),
            UnsupportedMode::ImageGeneration => json!({
                "mode": "image_generation",
                "error": image_generation_rejected(),
            }),
        })
        .collect();
    json!({"modes": results})
}

fn batch_embedding_rejected() -> bool {
    let catalog = BundledCatalog::default().get();
    get_vertex_url(
        &catalog,
        &VertexUrlInput {
            mode: UrlMode::BatchEmbedding,
            model: "gemini-embedding-2",
            stream: None,
            project: Some("proj"),
            location: Some("us-central1"),
            api_version: ApiVersion::V1,
        },
    )
    .is_err()
}

fn image_generation_rejected() -> bool {
    get_gemini_url(&GeminiUrlInput {
        mode: UrlMode::ImageGeneration,
        model: "gemini-2.5-flash",
        stream: None,
    })
    .is_err()
}
