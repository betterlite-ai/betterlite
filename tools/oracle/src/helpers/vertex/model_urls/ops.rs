use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_vertex::model_urls::base_url::get_vertex_base_url;
use betterlite_vertex::model_urls::embedding::{EmbeddingUrlInput, get_embedding_url};
use betterlite_vertex::model_urls::gemini::{GeminiUrlInput, get_gemini_url};
use betterlite_vertex::model_urls::model_name::{
    forward_gemini_call_id, get_fine_tuned_endpoint_id, get_model_for_vertex_ai_url,
    get_vertex_base_model_name, is_gemini_3_or_newer,
};
use betterlite_vertex::model_urls::vertex::{VertexUrlInput, get_vertex_url};
use betterlite_vertex::model_urls::{ApiVersion, UrlMode, VertexUrl};
use serde_json::{Value, json};

use super::input::{CaseError, Input, UnsupportedMode, UrlCase, UrlKind};

pub(super) fn run(input: &Input) -> Value {
    match input {
        Input::BaseUrl { locations } => base_urls(locations),
        Input::Urls { cases } => urls(cases),
        Input::ModelPredicates { models } => model_predicates(models),
        Input::UnsupportedModes { modes } => unsupported_modes(modes),
    }
}

fn url_value(result: Result<VertexUrl, CaseError>) -> Value {
    result.map_or_else(
        |_error| json!({"error": true}),
        |url| json!({"url": url.url, "endpoint": url.endpoint.as_str()}),
    )
}

fn urls(cases: &[UrlCase]) -> Value {
    let catalog = BundledCatalog::default().get();
    let urls: Vec<Value> = cases
        .iter()
        .map(|case| url_value(case.build(&catalog)))
        .collect();
    json!({"urls": urls})
}

impl UrlCase {
    fn build(&self, catalog: &ModelCatalog) -> Result<VertexUrl, CaseError> {
        match self.kind {
            UrlKind::Vertex => get_vertex_url(
                catalog,
                &VertexUrlInput {
                    mode: self.required_mode()?,
                    model: &self.model,
                    stream: self.stream,
                    project: self.project.as_deref(),
                    location: self.location.as_deref(),
                    api_version: self.required_version()?,
                },
            )
            .map_err(CaseError::Url),
            UrlKind::Embedding => get_embedding_url(
                catalog,
                &EmbeddingUrlInput {
                    model: &self.model,
                    project: self.project.as_deref(),
                    location: self.location.as_deref(),
                    api_version: self.required_version()?,
                },
            )
            .map_err(CaseError::Url),
            UrlKind::Gemini => get_gemini_url(&GeminiUrlInput {
                mode: self.required_mode()?,
                model: &self.model,
                stream: self.stream,
            })
            .map_err(CaseError::Url),
        }
    }

    fn required_mode(&self) -> Result<UrlMode, CaseError> {
        let mode = self.mode.ok_or_else(|| CaseError::MissingMode {
            kind: self.kind.name(),
        })?;
        Ok(mode.into_mode())
    }

    fn required_version(&self) -> Result<ApiVersion, CaseError> {
        let version = self.api_version.ok_or_else(|| CaseError::MissingVersion {
            kind: self.kind.name(),
        })?;
        Ok(version.into_version())
    }
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

fn model_predicates(models: &[String]) -> Value {
    let entries: Vec<Value> = models
        .iter()
        .map(|model| {
            json!({
                "base_name": get_vertex_base_model_name(model),
                "fine_tuned_id": get_fine_tuned_endpoint_id(model),
                "url_model": get_model_for_vertex_ai_url(model),
                "newer": is_gemini_3_or_newer(model),
                "forward": forward_gemini_call_id(model),
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
