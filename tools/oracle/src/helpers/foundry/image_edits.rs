use betterlite_foundry::image_edits::error::ImageEditError;
use betterlite_foundry::image_edits::mai;
use betterlite_foundry::image_edits::selection::ImageEditVariant;
use betterlite_foundry::image_generation::error::FoundryImageError;
use betterlite_foundry::image_generation::flux::supported_params;
use betterlite_foundry::mai_images::error::MaiImageError;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Map, Value, json};

mod input;
mod replies;
mod steps;
mod tests;

use input::Input;
use replies::reply_step;
use steps::{auth_step, transform_step, url_step};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answer = Map::new();
        insert(
            &mut answer,
            "models",
            input.models.as_deref(),
            |model: &String| Ok(model_step(model.as_str())),
        )?;
        insert(
            &mut answer,
            "supported",
            input.supported.as_deref(),
            |model: &String| Ok(supported_step(model.as_str())),
        )?;
        insert(&mut answer, "auths", input.auths.as_deref(), auth_step)?;
        insert(&mut answer, "urls", input.urls.as_deref(), url_step)?;
        insert(
            &mut answer,
            "transforms",
            input.transforms.as_deref(),
            |step| Ok(transform_step(step)),
        )?;
        insert(&mut answer, "replies", input.replies.as_deref(), |step| {
            Ok(reply_step(step))
        })?;
        Ok(Value::Object(answer))
    }
}

fn insert<T>(
    answer: &mut Map<String, Value>,
    key: &str,
    values: Option<&[T]>,
    step: fn(&T) -> Result<Value, Box<HelperError>>,
) -> Result<(), Box<HelperError>> {
    let Some(entries) = values else {
        return Ok(());
    };
    let mapped: Vec<Value> = entries.iter().map(step).collect::<Result<Vec<_>, _>>()?;
    answer.insert(key.to_owned(), json!(mapped));
    Ok(())
}

fn model_step(model: &str) -> Value {
    ImageEditVariant::select(model).map_or_else(
        |_| json!({"error": {"kind": "ValueError", "status": null, "provider": null}}),
        |variant| json!({"class": variant.class_name(), "multipart": variant.uses_multipart()}),
    )
}

// the FLUX 1 allowlist belongs to the OpenAI image edit config, so only the two owned allowlists are probed
fn supported_step(model: &str) -> Value {
    match ImageEditVariant::select(model) {
        Ok(ImageEditVariant::Flux2) => json!(supported_params(model)),
        Ok(ImageEditVariant::Mai) => json!(&mai::MAI_EDIT_OPENAI_PARAMS),
        Ok(ImageEditVariant::Flux1) | Err(_) => Value::Null,
    }
}

// the exception LiteLLM raises for each betterlite error, so the oracle compares the branch, not the text
fn error_value(error: &ImageEditError) -> Value {
    match error {
        ImageEditError::Params(inner) => params_failure(inner),
        ImageEditError::Mai(MaiImageError::NotJson { record, .. }) => json!({
            "error": {"kind": "OpenAIError", "status": record.status_code, "provider": null},
        }),
        ImageEditError::Mai(_) | ImageEditError::Projection(_) => json!({
            "error": {"kind": "ValueError", "status": error.status_code(), "provider": null},
        }),
        ImageEditError::UnsupportedModel { .. }
        | ImageEditError::ApiBaseRequired
        | ImageEditError::ApiVersionRequired
        | ImageEditError::Flux2PromptRequired
        | ImageEditError::Flux2ImageRequired
        | ImageEditError::Flux2NoReferenceImages
        | ImageEditError::Flux2TooManyReferenceImages { .. }
        | ImageEditError::UnsupportedImageType { .. }
        | ImageEditError::Url(_)
        | ImageEditError::Credentials(_)
        | ImageEditError::ReplyEncoding(_) => {
            json!({"error": {"kind": "ValueError", "status": null, "provider": null}})
        }
    }
}

fn params_failure(error: &FoundryImageError) -> Value {
    let kind = match error {
        FoundryImageError::InvalidSize { .. } => "BadRequestError",
        FoundryImageError::UnsupportedParams { .. } => "UnsupportedParamsError",
        FoundryImageError::ApiBaseRequired
        | FoundryImageError::WholeNumber { .. }
        | FoundryImageError::Decimal { .. }
        | FoundryImageError::GptParams(_) => "ValueError",
    };
    json!({
        "error": {
            "kind": kind,
            "status": error.status_code(),
            "provider": error.llm_provider(),
        },
    })
}
