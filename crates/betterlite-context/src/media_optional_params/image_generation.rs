use betterlite_models::controls::json_value::JsonValue;

use super::check::{FirstUnsupported, drop_unsupported_params};
use super::error::MediaParamsError;
use crate::non_default_params::Params;
use crate::non_default_params::extra_body::{
    ProviderSpecificRequest, add_provider_specific_params,
};
use crate::non_default_params::filters::select_non_default_params;
use crate::non_default_params::mapper::{DropPolicy, OptionalParamsMapper};
use crate::non_default_params::names::OPENAI_COMPATIBLE_PROVIDERS;

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

// Square is the answer for any size LiteLLM does not know.
#[must_use]
pub fn vertex_aspect_ratio(size: Option<&str>) -> &'static str {
    match size {
        Some("1792x1024") => "16:9",
        Some("1024x1792") => "9:16",
        Some(_) | None => "1:1",
    }
}

// Cloud-init kwargs only travel to the providers that read them.
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
        Some(_) => {
            return Err(MediaParamsError::WrongType {
                field: "n",
                expected: "an integer",
            });
        }
    }
    match params.get("size") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::Text(size)) => {
            let ratio = vertex_aspect_ratio(Some(size));
            optional_params.insert("aspectRatio".to_owned(), JsonValue::Text(ratio.to_owned()));
        }
        Some(_) => {
            return Err(MediaParamsError::WrongType {
                field: "size",
                expected: "a string",
            });
        }
    }
    Ok(optional_params)
}

fn unsupported_error(
    request: &ImageGenerationRequest<'_>,
    first: FirstUnsupported,
) -> MediaParamsError {
    MediaParamsError::ImageUnsupported {
        param: first.0,
        provider: request.custom_llm_provider.unwrap_or("None").to_owned(),
        model: request.model.unwrap_or("None").to_owned(),
    }
}

struct BranchOutput {
    optional_params: Params,
    dropped: Vec<String>,
}

enum Plan<'request> {
    Mapped {
        config: Option<&'request dyn OptionalParamsMapper>,
        supported: Vec<String>,
        drop_for_config: bool,
    },
    Passthrough,
    Empty,
}

fn plan_branch<'request>(
    request: &ImageGenerationRequest<'request>,
    provider_supported: Option<&[String]>,
    drop: bool,
) -> Result<Plan<'request>, MediaParamsError> {
    match (request.provider_config, request.custom_llm_provider) {
        (Some(config), _) => Ok(Plan::Mapped {
            config: Some(config),
            supported: provider_supported.unwrap_or_default().to_vec(),
            drop_for_config: drop,
        }),
        (None, Some("bedrock")) => {
            let config = request
                .bedrock_config
                .ok_or(MediaParamsError::MissingBedrockConfig)?;
            Ok(Plan::Mapped {
                config: Some(config),
                supported: config.supported_openai_params(request.model.unwrap_or_default()),
                drop_for_config: false,
            })
        }
        (None, Some("vertex_ai")) => Ok(Plan::Mapped {
            config: None,
            supported: vec!["n".to_owned(), "size".to_owned()],
            drop_for_config: false,
        }),
        (None, Some(name))
            if matches!(name, "openai" | "azure")
                || OPENAI_COMPATIBLE_PROVIDERS.contains(&name) =>
        {
            Ok(Plan::Passthrough)
        }
        (None, Some(_) | None) => Ok(Plan::Empty),
    }
}

fn run_branch(
    request: &ImageGenerationRequest<'_>,
    mut non_default_params: Params,
    provider_supported: Option<&[String]>,
) -> Result<BranchOutput, MediaParamsError> {
    let drop =
        request.drop_policy.litellm_drop_params || request.drop_policy.drop_params == Some(true);
    match plan_branch(request, provider_supported, drop)? {
        Plan::Passthrough => Ok(BranchOutput {
            optional_params: non_default_params,
            dropped: Vec::new(),
        }),
        Plan::Empty => Ok(BranchOutput {
            optional_params: Params::new(),
            dropped: Vec::new(),
        }),
        Plan::Mapped {
            config,
            supported,
            drop_for_config,
        } => {
            let dropped = drop_unsupported_params(&mut non_default_params, &supported, drop)
                .map_err(|first| unsupported_error(request, first))?;
            let optional_params = match config {
                Some(mapper) => mapper.map_openai_params(
                    &non_default_params,
                    Params::new(),
                    request.model.unwrap_or_default(),
                    drop_for_config,
                )?,
                None => vertex_optional_params(request.params)?,
            };
            Ok(BranchOutput {
                optional_params,
                dropped,
            })
        }
    }
}

fn drop_empty_values(optional_params: &mut Params) {
    optional_params.retain(|_, value| match value {
        JsonValue::Null => false,
        JsonValue::Object(fields) => !fields.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) => true,
    });
}

pub fn get_optional_params_image_gen(
    request: &ImageGenerationRequest<'_>,
) -> Result<Params, MediaParamsError> {
    let provider = request.custom_llm_provider;
    let mut passed_params = request.params.clone();
    passed_params.retain(|key, _| !kwarg_is_dropped(key, provider));
    let provider_supported = request
        .provider_config
        .map(|config| config.supported_openai_params(request.model.unwrap_or_default()));
    let default_names: Vec<&str> = IMAGE_DEFAULTS
        .iter()
        .copied()
        .chain(provider_supported.iter().flatten().map(String::as_str))
        .collect();
    let non_default_params = select_non_default_params(
        &passed_params,
        &default_names,
        request.additional_drop_params,
    );
    let branch = run_branch(request, non_default_params, provider_supported.as_deref())?;
    for key in &branch.dropped {
        passed_params.remove(key);
    }
    let openai_params: Vec<&str> = provider_supported
        .as_ref()
        .map_or(default_names, |supported| {
            supported.iter().map(String::as_str).collect()
        });
    let mut optional_params = add_provider_specific_params(
        branch.optional_params,
        &ProviderSpecificRequest {
            passed_params: &passed_params,
            custom_llm_provider: provider.unwrap_or_default(),
            openai_params: &openai_params,
            additional_drop_params: request.additional_drop_params,
        },
    )?;
    drop_empty_values(&mut optional_params);
    Ok(optional_params)
}
