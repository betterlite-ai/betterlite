use crate::image_models::params::{Field, Params};

use super::async_invoke::wrap_async_invoke_request;
use super::data_url::parse_nova_data_url;
use super::error::NovaEmbeddingsError;

pub const SCHEMA_VERSION: &str = "nova-multimodal-embed-v1";
pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &["dimensions"];
const DEFAULT_PURPOSE: &str = "GENERIC_INDEX";
const DEFAULT_DIMENSION: i64 = 3072;

#[must_use]
pub fn map_openai_params(non_default_params: &Params, mut optional_params: Params) -> Params {
    for (key, value) in non_default_params {
        if key == "dimensions" {
            optional_params.insert("embedding_dimension".to_owned(), value.clone());
        }
    }
    optional_params
}

pub fn transform_request(
    input: &str,
    inference_params: &Params,
    async_invoke_route: bool,
    model_id: Option<&str>,
    output_s3_uri: Option<&str>,
) -> Result<Params, NovaEmbeddingsError> {
    let mut embedding_params = inference_params.clone();
    embedding_params.shift_remove("output_s3_uri");
    apply_dimension_override(&mut embedding_params);
    insert_required_defaults(&mut embedding_params);

    let has_modality = ["text", "image", "video", "audio"]
        .iter()
        .any(|key| embedding_params.contains_key(*key));
    if !has_modality {
        let (key, field) = infer_modality(input)?;
        embedding_params.insert(key, field);
    }

    let request = build_request(async_invoke_route, embedding_params);
    maybe_wrap_async(request, async_invoke_route, model_id, output_s3_uri)
}

fn apply_dimension_override(embedding_params: &mut Params) {
    if let Some(value) = embedding_params.shift_remove("dimensions") {
        embedding_params.insert("embeddingDimension".to_owned(), value);
        return;
    }
    if let Some(value) = embedding_params.shift_remove("embedding_dimension") {
        embedding_params.insert("embeddingDimension".to_owned(), value);
    }
}

fn insert_required_defaults(embedding_params: &mut Params) {
    embedding_params
        .entry("embeddingPurpose".to_owned())
        .or_insert_with(|| Field::Text(DEFAULT_PURPOSE.to_owned()));
    embedding_params
        .entry("embeddingDimension".to_owned())
        .or_insert_with(|| Field::Int(DEFAULT_DIMENSION));
}

fn build_request(async_invoke_route: bool, embedding_params: Params) -> Params {
    let task_type = if async_invoke_route {
        "SEGMENTED_EMBEDDING"
    } else {
        "SINGLE_EMBEDDING"
    };
    let params_key = if async_invoke_route {
        "segmentedEmbeddingParams"
    } else {
        "singleEmbeddingParams"
    };

    let mut request = Params::new();
    request.insert(
        "schemaVersion".to_owned(),
        Field::Text(SCHEMA_VERSION.to_owned()),
    );
    request.insert("taskType".to_owned(), Field::Text(task_type.to_owned()));
    request.insert(params_key.to_owned(), Field::Mapping(embedding_params));
    request
}

fn maybe_wrap_async(
    request: Params,
    async_invoke_route: bool,
    model_id: Option<&str>,
    output_s3_uri: Option<&str>,
) -> Result<Params, NovaEmbeddingsError> {
    if !async_invoke_route {
        return Ok(request);
    }
    let Some(truthy_model_id) = model_id.filter(|value| !value.is_empty()) else {
        return Ok(request);
    };
    wrap_async_invoke_request(request, truthy_model_id, output_s3_uri)
}

fn infer_modality(input: &str) -> Result<(String, Field), NovaEmbeddingsError> {
    if input.starts_with("data:") {
        let parsed = parse_nova_data_url(input)?;
        if let Some((key, format)) = media_modality(&parsed.media_type) {
            return Ok((key.to_owned(), media_field(format, parsed.data)));
        }
        return Ok(text_entry(input));
    }
    if input.starts_with("s3://") {
        return Ok(s3_entry(input));
    }
    Ok(text_entry(input))
}

fn media_modality(media_type: &str) -> Option<(&'static str, String)> {
    let (type_part, rest) = media_type.split_once('/')?;
    let key = match type_part {
        "image" => "image",
        "video" => "video",
        "audio" => "audio",
        _ => return None,
    };
    let format_raw = rest.split('/').next().unwrap_or(rest);
    let mut format = format_raw.to_lowercase();
    if key == "image" && format == "jpg" {
        "jpeg".clone_into(&mut format);
    }
    Some((key, format))
}

fn media_field(format: String, data: String) -> Field {
    let mut source = Params::new();
    source.insert("bytes".to_owned(), Field::Text(data));
    let mut mapping = Params::new();
    mapping.insert("format".to_owned(), Field::Text(format));
    mapping.insert("source".to_owned(), Field::Mapping(source));
    Field::Mapping(mapping)
}

fn text_entry(input: &str) -> (String, Field) {
    let mut mapping = Params::new();
    mapping.insert("value".to_owned(), Field::Text(input.to_owned()));
    mapping.insert("truncationMode".to_owned(), Field::Text("END".to_owned()));
    ("text".to_owned(), Field::Mapping(mapping))
}

fn s3_entry(input: &str) -> (String, Field) {
    let mut uri = Params::new();
    uri.insert("uri".to_owned(), Field::Text(input.to_owned()));
    let mut location = Params::new();
    location.insert("s3Location".to_owned(), Field::Mapping(uri));
    let mut mapping = Params::new();
    mapping.insert("source".to_owned(), Field::Mapping(location));
    mapping.insert("truncationMode".to_owned(), Field::Text("END".to_owned()));
    ("text".to_owned(), Field::Mapping(mapping))
}
