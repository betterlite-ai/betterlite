use betterlite_bedrock::canvas_results::COST_HEADER;
use betterlite_bedrock::image_call::plan::ImageEnvironment;
use betterlite_bedrock::image_edits::plan::{EditEnvironment, EditImage, EditPlan};
use betterlite_bedrock::image_edits::response::EditReply;
use betterlite_bedrock::image_models::params::Params;
use betterlite_bedrock::image_models::response::ModelResponse;
use betterlite_bedrock::transcription::error::CallError;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_provider::bedrock_image_dispatch::BedrockImageResult;
use betterlite_provider::bedrock_image_dispatch::call::{
    BedrockImageOperation, EditOperation, GenerateOperation,
};
use betterlite_provider::bedrock_image_dispatch::test_support::Catalog;
use betterlite_provider::bedrock_other_dispatch::BedrockOtherResult;
use betterlite_provider::bedrock_other_dispatch::call::BedrockOtherOperation;
use betterlite_provider::bedrock_other_dispatch::test_support::binding::Recording;
use betterlite_provider::bedrock_other_dispatch::test_support::{NOW_SECONDS, instant};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::super::harness::{Handler, Harness, Outcome, exchange};
use super::super::model::{Failed, Input, Probe, extra_pairs, from_params};
use super::row;
use crate::openai_media_dispatch::failure;

pub(super) async fn image_row(input: &Input, probe: &Probe, edit: bool) -> Result<Value, Failed> {
    if edit {
        edit_row(input, probe).await
    } else {
        generation_row(probe).await
    }
}

// One Canvas edit: the source images, the image params and the plan the catalog resolves
async fn edit_row(input: &Input, probe: &Probe) -> Result<Value, Failed> {
    let extra = extra_pairs(probe);
    let key = probe.api_key.clone().map(SecretString::from);
    let images: Vec<EditImage> = probe.images.iter().cloned().map(EditImage::Text).collect();
    let catalog = Catalog::empty().map_err(|error| failure("Catalog", error))?;
    let context = catalog.context();
    let plan = EditPlan {
        model_cost: &input.model_cost,
        resolution: &context,
    };
    let now = instant(NOW_SECONDS).map_err(|error| failure("Instant", error))?;
    let operation = BedrockOtherOperation::Image(BedrockImageOperation::Edit(EditOperation {
        model: Some(&probe.model),
        prompt: probe.prompt.as_deref(),
        images: &images,
        optional_params: from_params::<ImageParams>(probe)?,
        extra_headers: extra.as_deref(),
        api_base: probe.api_base.as_deref(),
        api_key: key.as_ref(),
        timeout: None,
        aimage_edit: false,
        environment: EditEnvironment::default(),
        now,
        plan,
    }));
    run_image(probe, operation).await
}

// One Canvas generation: the prompt and the generation params, with no source image
async fn generation_row(probe: &Probe) -> Result<Value, Failed> {
    let extra = extra_pairs(probe);
    let key = probe.api_key.clone().map(SecretString::from);
    let now = instant(NOW_SECONDS).map_err(|error| failure("Instant", error))?;
    let operation =
        BedrockOtherOperation::Image(BedrockImageOperation::Generate(GenerateOperation {
            model: Some(&probe.model),
            prompt: probe.prompt.as_deref().unwrap_or_default(),
            optional_params: from_params::<Params>(probe)?,
            extra_headers: extra.as_deref(),
            api_base: probe.api_base.as_deref(),
            api_key: key.as_ref(),
            timeout: None,
            aimg_generation: false,
            environment: ImageEnvironment::default(),
            now,
            created: NOW_SECONDS,
        }));
    run_image(probe, operation).await
}

// The shared tail: one exchange, then the images the row compares
async fn run_image(
    probe: &Probe,
    operation: BedrockOtherOperation<'_, '_, '_, Handler, Recording>,
) -> Result<Value, Failed> {
    let harness = Harness::new(probe)?;
    let outcome: Outcome<CallError> = exchange::<Handler, Recording>(&harness, operation).await?;
    let value = match &outcome.result {
        Ok(BedrockOtherResult::Image(result)) => image_value(result),
        Ok(_other) => Value::Null,
        Err(_reported) => Value::Null,
    };
    row(&outcome, value, None)
}

// Both arms answer base64 images; only an edit reply carries the cost header
fn image_value(result: &BedrockImageResult) -> Value {
    match result {
        BedrockImageResult::Generate(response) => json!({
            "images": generation_images(response),
            "cost": Value::Null,
        }),
        BedrockImageResult::Edit(reply) => json!({
            "images": edit_images(reply),
            "cost": reply.additional_headers.get(COST_HEADER),
        }),
    }
}

fn generation_images(response: &ModelResponse) -> Value {
    let mut values = Vec::with_capacity(response.data.len());
    for image in &response.data {
        values.push(Value::String(image.b64_json.clone()));
    }
    Value::Array(values)
}

fn edit_images(reply: &EditReply) -> Value {
    let mut values = Vec::new();
    for image in reply.response.data.iter().flatten() {
        if let Some(b64) = image.b64_json.as_deref() {
            values.push(Value::String(b64.to_owned()));
        }
    }
    Value::Array(values)
}
