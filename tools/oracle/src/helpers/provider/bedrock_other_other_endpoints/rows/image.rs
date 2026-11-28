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
use betterlite_provider::bedrock_other_dispatch::BedrockOtherResult;
use betterlite_provider::bedrock_other_dispatch::call::BedrockOtherOperation;
use betterlite_provider::bedrock_other_dispatch::test_support::NOW_SECONDS;
use betterlite_provider::bedrock_other_dispatch::test_support::binding::Recording;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::super::harness::{Harness, Outcome, empty_catalog, exchange, now};
use super::super::model::{Failed, Input, Probe, Unavailable, api_key, extra_pairs, from_params};
use super::row;

pub(super) async fn image_row(input: &Input, probe: &Probe, edit: bool) -> Result<Value, Failed> {
    let extra = extra_pairs(probe);
    let key = api_key(probe);
    let images: Vec<EditImage> = probe.images.iter().cloned().map(EditImage::Text).collect();
    let catalog = empty_catalog()?;
    let context = catalog.context();
    let plan = EditPlan {
        model_cost: &input.model_cost,
        resolution: &context,
    };
    let operation = if edit {
        edit_operation(probe, &images, plan, extra.as_deref(), key.as_ref())?
    } else {
        generation_operation(probe, extra.as_deref(), key.as_ref())?
    };
    let harness = Harness::new(probe)?;
    let outcome: Outcome<CallError> =
        exchange::<Unavailable, Recording>(&harness, operation).await?;
    let value = match &outcome.result {
        Ok(BedrockOtherResult::Image(result)) => image_value(result),
        Ok(_other) => Value::Null,
        Err(_reported) => Value::Null,
    };
    row(&outcome, value, None)
}

fn edit_operation<'call, 'cost, 'context>(
    probe: &'call Probe,
    images: &'call [EditImage],
    plan: EditPlan<'cost, 'context>,
    extra: Option<&'call [(String, String)]>,
    key: Option<&'call SecretString>,
) -> Result<BedrockOtherOperation<'call, 'cost, 'context, Unavailable, Recording>, Failed> {
    let optional_params: ImageParams = from_params(probe)?;
    Ok(BedrockOtherOperation::Image(BedrockImageOperation::Edit(
        EditOperation {
            model: Some(&probe.model),
            prompt: probe.prompt.as_deref(),
            images,
            optional_params,
            extra_headers: extra,
            api_base: probe.api_base.as_deref(),
            api_key: key,
            timeout: None,
            aimage_edit: false,
            environment: EditEnvironment::default(),
            now: now()?,
            plan,
        },
    )))
}

fn generation_operation<'call>(
    probe: &'call Probe,
    extra: Option<&'call [(String, String)]>,
    key: Option<&'call SecretString>,
) -> Result<BedrockOtherOperation<'call, 'call, 'call, Unavailable, Recording>, Failed> {
    let optional_params: Params = from_params(probe)?;
    let prompt = probe.prompt.as_deref().unwrap_or_default();
    Ok(BedrockOtherOperation::Image(
        BedrockImageOperation::Generate(GenerateOperation {
            model: Some(&probe.model),
            prompt,
            optional_params,
            extra_headers: extra,
            api_base: probe.api_base.as_deref(),
            api_key: key,
            timeout: None,
            aimg_generation: false,
            environment: ImageEnvironment::default(),
            now: now()?,
            created: NOW_SECONDS,
        }),
    ))
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
