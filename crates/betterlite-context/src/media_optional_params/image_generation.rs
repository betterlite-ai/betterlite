use betterlite_models::controls::json_value::JsonValue;

use super::check::drop_unsupported_params;
use super::error::MediaParamsError;
use crate::non_default_params::Params;
use crate::non_default_params::extra_body::{
    ProviderSpecificRequest, add_provider_specific_params,
};
use crate::non_default_params::filters::select_non_default_params;
use crate::non_default_params::mapper::{DropPolicy, OptionalParamsMapper};
use crate::non_default_params::names::is_openai_compatible_provider;

const IMAGE_DEFAULTS: &[&str] = &[
    "n",
    "quality",
    "response_format",
    "size",
    "style",
    "user",
    "imageConfig",
    "tools",
    "web_search_options",
];

pub struct ImageGenerationRequest<'request> {
    pub model: Option<&'request str>,
    pub custom_llm_provider: Option<&'request str>,
    pub params: &'request Params,
    pub additional_drop_params: &'request [String],
    pub drop_policy: DropPolicy,
    pub provider_config: Option<&'request dyn OptionalParamsMapper>,
    pub bedrock_config: Option<&'request dyn OptionalParamsMapper>,
}

#[must_use]
pub fn vertex_aspect_ratio(size: Option<&str>) -> &'static str {
    match size {
        Some("1792x1024") => "16:9",
        Some("1024x1792") => "9:16",
        Some(_) | None => "1:1",
    }
}

fn kwarg_is_dropped(key: &str, custom_llm_provider: Option<&str>) -> bool {
    if key.starts_with("aws_") {
        return !matches!(custom_llm_provider, Some("bedrock" | "sagemaker"));
    }
    if key == "hf_model_name" {
        return custom_llm_provider != Some("sagemaker");
    }
    key.starts_with("vertex_")
        && !matches!(custom_llm_provider, Some("vertex_ai" | "vertex_ai_beta"))
}

fn vertex_optional_params(params: &Params) -> Result<Params, MediaParamsError> {
    let mut optional_params = Params::new();
    match params.get("n") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::Number(count)) if count.is_i64() || count.is_u64() => {
            optional_params.insert("sampleCount".to_owned(), JsonValue::Number(count.clone()));
        }
        Some(_) => return Err(MediaParamsError::WrongType("n", "an integer")),
    }
    match params.get("size") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::Text(size)) => {
            let ratio = vertex_aspect_ratio(Some(size));
            optional_params.insert("aspectRatio".to_owned(), JsonValue::Text(ratio.to_owned()));
        }
        Some(_) => return Err(MediaParamsError::WrongType("size", "a string")),
    }
    Ok(optional_params)
}

fn drop_unsupported(
    request: &ImageGenerationRequest<'_>,
    non_default_params: &mut Params,
    supported: &[String],
    drop: bool,
) -> Result<Vec<String>, MediaParamsError> {
    drop_unsupported_params(non_default_params, supported, drop).map_err(|first| {
        MediaParamsError::ImageUnsupported {
            param: first.0,
            provider: request.custom_llm_provider.unwrap_or("None").to_owned(),
            model: request.model.unwrap_or("None").to_owned(),
        }
    })
}

fn branch_params(
    request: &ImageGenerationRequest<'_>,
    supported_by_config: Option<&[String]>,
    mut non_default_params: Params,
) -> Result<(Params, Vec<String>), MediaParamsError> {
    let policy = request.drop_policy;
    let drop = policy.litellm_drop_params || policy.drop_params == Some(true);
    let model = request.model.unwrap_or_default();
    match (request.provider_config, request.custom_llm_provider) {
        (Some(config), _) => {
            let supported = supported_by_config.unwrap_or_default();
            let dropped = drop_unsupported(request, &mut non_default_params, supported, drop)?;
            let mapped =
                config.map_openai_params(&non_default_params, Params::new(), model, drop)?;
            Ok((mapped, dropped))
        }
        (None, Some("bedrock")) => {
            let config = request
                .bedrock_config
                .ok_or(MediaParamsError::MissingBedrockConfig)?;
            let supported = config.supported_openai_params(model);
            let dropped = drop_unsupported(request, &mut non_default_params, &supported, drop)?;
            let mapped =
                config.map_openai_params(&non_default_params, Params::new(), model, false)?;
            Ok((mapped, dropped))
        }
        (None, Some("vertex_ai")) => {
            let supported = ["n".to_owned(), "size".to_owned()];
            let dropped = drop_unsupported(request, &mut non_default_params, &supported, drop)?;
            Ok((vertex_optional_params(request.params)?, dropped))
        }
        (None, Some(name))
            if matches!(name, "openai" | "azure") || is_openai_compatible_provider(name) =>
        {
            Ok((non_default_params, Vec::new()))
        }
        (None, Some(_) | None) => Ok((Params::new(), Vec::new())),
    }
}

pub fn get_optional_params_image_gen(
    request: &ImageGenerationRequest<'_>,
) -> Result<Params, MediaParamsError> {
    let mut passed_params = request.params.clone();
    passed_params.retain(|key, _| !kwarg_is_dropped(key, request.custom_llm_provider));
    let supported_by_config = request
        .provider_config
        .map(|config| config.supported_openai_params(request.model.unwrap_or_default()));
    let default_names: Vec<&str> = IMAGE_DEFAULTS
        .iter()
        .copied()
        .chain(supported_by_config.iter().flatten().map(String::as_str))
        .collect();
    let non_default_params = select_non_default_params(
        &passed_params,
        &default_names,
        request.additional_drop_params,
    );
    let (mut optional_params, dropped) =
        branch_params(request, supported_by_config.as_deref(), non_default_params)?;
    for key in &dropped {
        passed_params.remove(key);
    }
    let openai_params: Vec<&str> = supported_by_config.as_ref().map_or(default_names, |names| {
        names.iter().map(String::as_str).collect()
    });
    optional_params = add_provider_specific_params(
        optional_params,
        &ProviderSpecificRequest {
            passed_params: &passed_params,
            custom_llm_provider: request.custom_llm_provider.unwrap_or_default(),
            openai_params: &openai_params,
            additional_drop_params: request.additional_drop_params,
        },
    )?;
    optional_params.retain(|_, value| match value {
        JsonValue::Null => false,
        JsonValue::Object(fields) => !fields.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) => true,
    });
    Ok(optional_params)
}
