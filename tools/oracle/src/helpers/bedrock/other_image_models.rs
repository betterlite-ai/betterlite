use betterlite_bedrock::image_models::error::ImageModelsError;
use betterlite_bedrock::image_models::family::{self, Family};
use betterlite_bedrock::image_models::params::{Field, Params};
use betterlite_bedrock::image_models::response::{
    ImagesOnlyResponse, ModelResponse, Stability1Response, Stability3Response, TitanResponse,
    optional_images, stability1_images, titan_images,
};
use betterlite_bedrock::image_models::stability1::{self, ClassSettings};
use betterlite_bedrock::image_models::{nova, stability3, titan};
use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult};

mod calls;

use calls::{
    ArtifactsCall, FamilyName, ProviderErrorCall, SelectionCall, SettingsCall, SizeQualityCall,
    Stability1Call, TextParamsCall, TitanCall,
};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    TitanImageBody(TitanCall),
    Stability1TextPrompts(Stability1Call),
    Stability3Prompt(TextParamsCall),
    NovaCanvasTaskBody(TextParamsCall),
    SizeNQualityMap(SizeQualityCall),
    ArtifactsToB64Results(ArtifactsCall),
    ImageProviderError(ProviderErrorCall),
    ImageFamilySelection(SelectionCall),
}

#[derive(Debug, Error)]
enum Failure {
    #[error(transparent)]
    ImageModels(#[from] ImageModelsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = match input {
            Input::TitanImageBody(call) => titan_image_body(call),
            Input::Stability1TextPrompts(call) => stability1_text_prompts(call),
            Input::Stability3Prompt(call) => stability3_prompt(call),
            Input::NovaCanvasTaskBody(call) => nova_canvas_task_body(call),
            Input::SizeNQualityMap(call) => size_n_quality_map(call),
            Input::ArtifactsToB64Results(call) => artifacts_to_b64_results(call),
            Input::ImageProviderError(call) => image_provider_error(call),
            Input::ImageFamilySelection(call) => Ok(image_family_selection(call)),
        };
        Ok(output?)
    }
}

fn titan_image_body(call: &TitanCall) -> Result<Value, Failure> {
    let non_default = map_to_params(&call.non_default)?;
    let optional_params = map_to_params(&call.optional_params)?;
    let mapped = titan::map_openai_params(&non_default, optional_params)?;
    let body = titan::transform_request_body(&call.text, mapped)?;
    Ok(Value::Object(params_to_map(&body)?))
}

fn stability1_text_prompts(call: &Stability1Call) -> Result<Value, Failure> {
    let non_default = map_to_params(&call.non_default)?;
    let optional_params = map_to_params(&call.optional_params)?;
    let mapped = stability1::map_openai_params(&non_default, optional_params)?;
    let settings = class_settings(&call.settings)?;
    let body = stability1::transform_request_body(&call.text, &mapped, &settings);
    Ok(Value::Object(params_to_map(&body)?))
}

fn class_settings(call: &SettingsCall) -> Result<ClassSettings, Failure> {
    let cfg_scale = optional_field(call.cfg_scale.as_ref())?;
    let seed = optional_field(call.seed.as_ref())?;
    let steps = optional_field(call.steps.as_ref())?;
    let width = optional_field(call.width.as_ref())?;
    let height = optional_field(call.height.as_ref())?;
    Ok(ClassSettings {
        cfg_scale,
        seed,
        steps,
        width,
        height,
    })
}

fn optional_field(value: Option<&Value>) -> Result<Option<Field>, Failure> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let field: Field = serde_json::from_value(raw.clone())?;
    Ok(Some(field))
}

fn stability3_prompt(call: &TextParamsCall) -> Result<Value, Failure> {
    let optional_params = map_to_params(&call.optional_params)?;
    let body = stability3::transform_request_body(&call.text, optional_params)?;
    Ok(Value::Object(params_to_map(&body)?))
}

fn nova_canvas_task_body(call: &TextParamsCall) -> Result<Value, Failure> {
    let optional_params = map_to_params(&call.optional_params)?;
    let body = nova::transform_request_body(&call.text, optional_params)?;
    Ok(Value::Object(params_to_map(&body)?))
}

fn size_n_quality_map(call: &SizeQualityCall) -> Result<Value, Failure> {
    let non_default = map_to_params(&call.non_default)?;
    let optional_params = map_to_params(&call.optional_params)?;
    let mapped = match family_or_stability1(call.family) {
        Family::Titan => titan::map_openai_params(&non_default, optional_params)?,
        Family::Stability1 => stability1::map_openai_params(&non_default, optional_params)?,
        Family::Stability3 => optional_params,
        Family::Nova => nova::map_openai_params(&non_default, optional_params)?,
    };
    Ok(Value::Object(params_to_map(&mapped)?))
}

fn artifacts_to_b64_results(call: &ArtifactsCall) -> Result<Value, Failure> {
    let response = Value::Object(call.response_body.clone());
    let images = match family_or_stability1(call.family) {
        Family::Titan => titan_images(&serde_json::from_value::<TitanResponse>(response)?),
        Family::Stability1 => {
            stability1_images(&serde_json::from_value::<Stability1Response>(response)?)
        }
        Family::Stability3 => {
            let typed: Stability3Response = serde_json::from_value(response)?;
            optional_images(&ImagesOnlyResponse {
                images: typed.images,
            })
        }
        Family::Nova => optional_images(&serde_json::from_value::<ImagesOnlyResponse>(response)?),
    };
    let model_response = ModelResponse {
        created: Some(42),
        data: images,
    };
    Ok(serde_json::json!({
        "created": model_response.created,
        "data": model_response
            .data
            .into_iter()
            .map(|image| serde_json::json!({"b64_json": image.b64_json}))
            .collect::<Vec<Value>>(),
    }))
}

fn image_provider_error(call: &ProviderErrorCall) -> Result<Value, Failure> {
    if let Some(response_body) = &call.response_body {
        let response: Stability3Response =
            serde_json::from_value(Value::Object(response_body.clone()))?;
        stability3::check_finish_reasons(&response)?;
        let images = optional_images(&ImagesOnlyResponse {
            images: response.images,
        });
        return Ok(serde_json::json!({
            "data": images
                .into_iter()
                .map(|image| serde_json::json!({"b64_json": image.b64_json}))
                .collect::<Vec<Value>>(),
        }));
    }
    let optional_params = map_to_params(&call.optional_params)?;
    let body = match family_or_stability1(call.family) {
        Family::Titan => titan::transform_request_body(&call.text, optional_params)?,
        Family::Nova => nova::transform_request_body(&call.text, optional_params)?,
        Family::Stability1 => stability1::transform_request_body(
            &call.text,
            &optional_params,
            &ClassSettings::default(),
        ),
        Family::Stability3 => stability3::transform_request_body(&call.text, optional_params)?,
    };
    Ok(Value::Object(params_to_map(&body)?))
}

fn image_family_selection(call: &SelectionCall) -> Value {
    let name = match family::select_family(call.model.as_deref()) {
        Family::Titan => "titan",
        Family::Stability1 => "stability1",
        Family::Stability3 => "stability3",
        Family::Nova => "nova",
    };
    Value::String(name.to_owned())
}

const fn family_or_stability1(name: Option<FamilyName>) -> Family {
    match name {
        Some(FamilyName::Titan) => Family::Titan,
        Some(FamilyName::Stability1) | None => Family::Stability1,
        Some(FamilyName::Stability3) => Family::Stability3,
        Some(FamilyName::Nova) => Family::Nova,
    }
}

fn map_to_params(map: &Map<String, Value>) -> Result<Params, Failure> {
    let value = Value::Object(map.clone());
    let params: Params = serde_json::from_value(value)?;
    Ok(params)
}

fn params_to_map(params: &Params) -> Result<Map<String, Value>, Failure> {
    let mut map = Map::new();
    for (key, field) in params {
        map.insert(key.clone(), serde_json::to_value(field)?);
    }
    Ok(map)
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let class = match &failure {
            Failure::ImageModels(ImageModelsError::Size(_)) => "ValueError",
            Failure::ImageModels(
                ImageModelsError::KeywordCollision(_)
                | ImageModelsError::NotAMapping(_)
                | ImageModelsError::JoinNotAString(..),
            ) => "TypeError",
            Failure::ImageModels(ImageModelsError::TaskTypeUnsupported(_)) => "NotImplementedError",
            Failure::ImageModels(ImageModelsError::Provider { .. }) => "BedrockError",
            Failure::ImageModels(ImageModelsError::Header(_)) => "UnicodeEncodeError",
            Failure::Json(_) => "KeyError",
        };
        let message =
            if let Failure::ImageModels(ImageModelsError::Provider { error, .. }) = &failure {
                error.error.message.clone()
            } else {
                failure.to_string()
            };
        Self::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message,
        })
    }
}

mod tests;
