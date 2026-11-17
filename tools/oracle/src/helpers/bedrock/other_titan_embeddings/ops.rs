use betterlite_bedrock::titan_embeddings::config::{
    ClassAttribute, ClassAttributes, TitanV2Settings, get_config,
};
use betterlite_bedrock::titan_embeddings::error::TitanEmbeddingsError;
use betterlite_bedrock::titan_embeddings::{cohere, g1, multimodal, response, v2};
use serde_json::{Value, json};
use thiserror::Error;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Json;

use super::input::{
    CohereV3InputType, G1Request, Input, MultimodalImage, ResponseProvider, TokenCountResponse,
    V2BinaryEmbedding, V2DimensionsNormalize,
};

pub(crate) fn run(input: &Input) -> HelperResult {
    match input {
        Input::TitanG1Request(case) => Ok(g1_request(case)),
        Input::TitanV2DimensionsNormalize(case) => v2_dimensions_normalize(case),
        Input::TitanV2BinaryEmbedding(case) => v2_binary_embedding(case),
        Input::TitanMultimodalImage(case) => Ok(multimodal_image(case)),
        Input::CohereV3InputType(case) => Ok(cohere_v3_input_type(case)),
        Input::TokenCountResponse(case) => token_count_response(case),
    }
}

// _transform_request reads only the input text, so the inference record never reaches the request
fn g1_request(case: &G1Request) -> Value {
    json!({
        "supported": g1::SUPPORTED_OPENAI_PARAMS,
        "mapped": case.optional_params,
        "non_default": case.non_default,
        "request": g1::transform_request(&case.input),
        "inference_params": case.inference_params,
        "configs": {
            "g1": get_config(&probe_attributes()),
            "v2": get_config(&probe_attributes()),
        },
    })
}

// the class settings live on the class across constructions, so each step is applied to the same state
fn v2_dimensions_normalize(case: &V2DimensionsNormalize) -> HelperResult {
    let mut state = TitanV2Settings::default();
    let mut configs = Vec::with_capacity(case.settings.len());
    for step in &case.settings {
        state.apply(step.normalize, step.dimensions);
        configs.push(serde_json::to_value(get_config(&state.attributes())).map_err(Failure::from)?);
    }
    let mapped = v2::map_openai_params(&case.non_default, case.optional_params.clone());
    let request =
        v2::transform_request(&case.input, &case.inference_params).map_err(Failure::from)?;
    Ok(json!({
        "configs": configs,
        "supported": v2::SUPPORTED_OPENAI_PARAMS,
        "mapped": mapped,
        "non_default": case.non_default,
        "request": request,
    }))
}

fn v2_binary_embedding(case: &V2BinaryEmbedding) -> HelperResult {
    let transformed =
        response::transform_response(&case.response_list, &case.model, v2::by_type_embedding)
            .map_err(Failure::from)?;
    Ok(serde_json::to_value(transformed).map_err(Failure::from)?)
}

fn multimodal_image(case: &MultimodalImage) -> Value {
    json!({
        "supported": multimodal::SUPPORTED_OPENAI_PARAMS,
        "mapped": multimodal::map_openai_params(&case.non_default, case.optional_params.clone()),
        "non_default": case.non_default,
        "request": multimodal::transform_request(&case.input, &case.inference_params),
        "inference_params": case.inference_params,
    })
}

fn cohere_v3_input_type(case: &CohereV3InputType) -> Value {
    let mapped = cohere::map_openai_params(&case.non_default, case.optional_params.clone());
    let request = cohere::transform_request(&case.model, &case.input, &mapped);
    json!({
        "supported": cohere::SUPPORTED_OPENAI_PARAMS,
        "mapped": mapped,
        "non_default": case.non_default,
        "request": request,
    })
}

fn token_count_response(case: &TokenCountResponse) -> HelperResult {
    let transformed = match &case.provider {
        ResponseProvider::G1 => response::transform_response(
            &case.response_list,
            &case.model,
            response::plain_embedding,
        ),
        ResponseProvider::V2 => {
            response::transform_response(&case.response_list, &case.model, v2::by_type_embedding)
        }
        ResponseProvider::Multimodal => multimodal::transform_response(
            &case.response_list,
            &case.model,
            case.batch_data.as_deref(),
        ),
    }
    .map_err(Failure::from)?;
    Ok(serde_json::to_value(transformed).map_err(Failure::from)?)
}

// dunders, None, a function, a builtin, a classmethod and a staticmethod all leave get_config
fn probe_attributes() -> ClassAttributes {
    let entries = [
        (
            "__hidden__",
            ClassAttribute::Value(Json::Text(String::from("dunder"))),
        ),
        (
            "_retained",
            ClassAttribute::Value(Json::Text(String::from("kept"))),
        ),
        (
            "_count",
            ClassAttribute::Value(Json::Number(serde_json::Number::from(3_i64))),
        ),
        ("_absent", ClassAttribute::Value(Json::Null)),
        ("_function", ClassAttribute::Function),
        ("_builtin", ClassAttribute::BuiltinFunction),
        ("_class_method", ClassAttribute::ClassMethod),
        ("_static_method", ClassAttribute::StaticMethod),
    ];
    entries
        .into_iter()
        .map(|(name, attribute)| (String::from(name), attribute))
        .collect()
}

#[derive(Debug, Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Embeddings(#[from] TitanEmbeddingsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        Self::new(HelperError {
            class: error_class(&failure).to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}

// the class is stripped before the comparison, so it only names the Python failure a reader would see
const fn error_class(failure: &Failure) -> &'static str {
    match failure {
        Failure::Json(_) => "ValueError",
        Failure::Embeddings(error) => match error {
            TitanEmbeddingsError::KeywordCollision(_)
            | TitanEmbeddingsError::NotIterable
            | TitanEmbeddingsError::NotSubscriptable => "TypeError",
            TitanEmbeddingsError::MissingField(_) => "KeyError",
            TitanEmbeddingsError::MalformedEmbedding(_)
            | TitanEmbeddingsError::MalformedTokenCount => "ValidationError",
            TitanEmbeddingsError::NoEmbeddingData(_)
            | TitanEmbeddingsError::IndexOverflow
            | TitanEmbeddingsError::TokenOverflow => "ValueError",
        },
    }
}
