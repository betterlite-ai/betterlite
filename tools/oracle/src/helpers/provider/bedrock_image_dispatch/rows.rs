use std::collections::BTreeMap;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::canvas_results::COST_HEADER;
use betterlite_bedrock::image_call::plan::ImageEnvironment;
use betterlite_bedrock::image_call::transport::ImageRequest;
use betterlite_bedrock::image_edits::plan::{EditEnvironment, EditImage, EditPlan};
use betterlite_bedrock::image_edits::response::EditReply;
use betterlite_bedrock::image_models::params::Params;
use betterlite_bedrock::image_models::response::ModelResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::bedrock_image_dispatch::BedrockImageResult;
use betterlite_provider::bedrock_image_dispatch::call::{
    BedrockImageOperation, EditOperation, GenerateOperation,
};
use betterlite_provider::bedrock_image_dispatch::error::BedrockImageError;
use betterlite_provider::bedrock_image_dispatch::test_support::{Catalog, NOW_SECONDS, instant};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::{Case, Input, Outcome, Probe, exchange};
use crate::openai_media_dispatch::{OTHER_OUTCOME, failure};

// Every probe runs the same production dispatch entry with its own fake transport
pub(super) async fn case_rows(input: &Input) -> HelperResult {
    let mut rows = Vec::with_capacity(input.probes.len());
    for probe in &input.probes {
        rows.push(probe_row(input, probe).await?);
    }
    Ok(Value::Array(rows))
}

async fn probe_row(input: &Input, probe: &Probe) -> Result<Value, Box<HelperError>> {
    if input.case.is_edit() {
        edit_row(input, probe).await
    } else {
        generation_row(probe).await
    }
}

async fn generation_row(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let prompt = probe
        .prompt
        .as_deref()
        .ok_or_else(|| failure("Prompt", "a generation probe carries a prompt"))?;
    let extra = header_pairs(probe.extra_headers.as_ref());
    let operation = GenerateOperation {
        model: Some(probe.model.as_str()),
        prompt,
        optional_params: generation_params(probe)?,
        extra_headers: extra.as_deref(),
        api_base: None,
        api_key: None,
        timeout: None,
        aimg_generation: false,
        environment: ImageEnvironment::default(),
        now: instant(NOW_SECONDS).map_err(|error| failure("Clock", error))?,
        created: NOW_SECONDS,
    };
    let outcome = exchange(
        BedrockImageOperation::Generate(operation),
        probe.reply.answer(),
        credentials(probe)?,
    )
    .await?;
    generation_value(&outcome, probe)
}

async fn edit_row(input: &Input, probe: &Probe) -> Result<Value, Box<HelperError>> {
    let images: Vec<EditImage> = probe.images.iter().cloned().map(EditImage::Text).collect();
    let catalog = Catalog::empty().map_err(|error| failure("Catalog", error))?;
    let context = catalog.context();
    let extra = header_pairs(probe.extra_headers.as_ref());
    let operation = EditOperation {
        model: Some(probe.model.as_str()),
        prompt: probe.prompt.as_deref(),
        images: &images,
        optional_params: probe.optional_params.clone(),
        extra_headers: extra.as_deref(),
        api_base: None,
        api_key: None,
        timeout: None,
        aimage_edit: false,
        environment: EditEnvironment::default(),
        now: instant(NOW_SECONDS).map_err(|error| failure("Clock", error))?,
        plan: EditPlan {
            model_cost: &input.model_cost,
            resolution: &context,
        },
    };
    let outcome = exchange(
        BedrockImageOperation::Edit(operation),
        probe.reply.answer(),
        credentials(probe)?,
    )
    .await?;
    edit_value(&outcome, probe)
}

// The generation leaf reads the typed parameter record the arm already normalised
fn generation_params(probe: &Probe) -> Result<Params, Box<HelperError>> {
    let value = serde_json::to_value(Json::Object(probe.optional_params.clone()))
        .map_err(|error| failure("Params", error))?;
    serde_json::from_value(value).map_err(|error| failure("Params", error))
}

// The probe's own aws keys are the identity the credential source hands over; the leaf pops the same keys
fn credentials(probe: &Probe) -> Result<Credentials, Box<HelperError>> {
    Ok(Credentials {
        access_key_id: SecretString::from(text_param(&probe.optional_params, "aws_access_key_id")?),
        secret_access_key: SecretString::from(text_param(
            &probe.optional_params,
            "aws_secret_access_key",
        )?),
        session_token: None,
    })
}

fn text_param(params: &ImageParams, name: &'static str) -> Result<String, Box<HelperError>> {
    match params.get(name) {
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(_) => Err(failure("Credentials", format!("{name} is not a string"))),
        None => Err(failure("Credentials", format!("{name} is missing"))),
    }
}

fn header_pairs(headers: Option<&BTreeMap<String, String>>) -> Option<Vec<(String, String)>> {
    headers.map(|pairs| {
        pairs
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    })
}

fn generation_value(outcome: &Outcome, probe: &Probe) -> Result<Value, Box<HelperError>> {
    let request = request_value(outcome, probe)?;
    match &outcome.result {
        Ok(BedrockImageResult::Generate(response)) => Ok(json!({
            "request": request,
            "ok": true,
            "status": Value::Null,
            "images": generation_images(response),
            "cost": Value::Null,
            "post_call": post_call(outcome),
        })),
        Ok(BedrockImageResult::Edit(_)) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => failure_value(outcome, probe, error),
    }
}

fn edit_value(outcome: &Outcome, probe: &Probe) -> Result<Value, Box<HelperError>> {
    let request = request_value(outcome, probe)?;
    match &outcome.result {
        Ok(BedrockImageResult::Edit(reply)) => Ok(json!({
            "request": request,
            "ok": true,
            "status": Value::Null,
            "images": edit_images(reply),
            "cost": reply.additional_headers.get(COST_HEADER),
            "post_call": post_call(outcome),
        })),
        Ok(BedrockImageResult::Generate(_)) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => failure_value(outcome, probe, error),
    }
}

fn failure_value(
    outcome: &Outcome,
    probe: &Probe,
    error: &BedrockImageError,
) -> Result<Value, Box<HelperError>> {
    Ok(json!({
        "request": request_value(outcome, probe)?,
        "ok": false,
        "status": error.status(),
        "images": Value::Array(Vec::new()),
        "cost": Value::Null,
        "post_call": post_call(outcome),
    }))
}

// The posted request both families compare: where it went, what it carried, and how it was signed
fn request_value(outcome: &Outcome, probe: &Probe) -> Result<Value, Box<HelperError>> {
    let Some(request) = outcome.requests.first() else {
        return Ok(json!({
            "url": Value::Null,
            "body": Value::Null,
            "authorization": Value::Null,
            "extra_header": Value::Null,
            "posted": 0,
        }));
    };
    let body: Json =
        serde_json::from_slice(&request.body).map_err(|error| failure("Fixture", error))?;
    let body = serde_json::to_value(body).map_err(|error| failure("Fixture", error))?;
    Ok(json!({
        "url": request.url.as_str(),
        "body": body,
        "authorization": authorization(request),
        "extra_header": extra_header(request, probe),
        "posted": outcome.requests.len(),
    }))
}

fn authorization(request: &ImageRequest) -> Value {
    match header(request, "authorization") {
        None => Value::Null,
        Some(value) if value.starts_with("AWS4-HMAC-SHA256") => Value::String("sigv4".to_owned()),
        Some(_) => Value::String("other".to_owned()),
    }
}

fn extra_header(request: &ImageRequest, probe: &Probe) -> Value {
    let Some(name) = probe
        .extra_headers
        .as_ref()
        .and_then(|headers| headers.keys().next())
    else {
        return Value::Null;
    };
    match header(request, name) {
        Some(value) => Value::String(value.to_owned()),
        None => Value::Null,
    }
}

fn header<'request>(request: &'request ImageRequest, name: &str) -> Option<&'request str> {
    request
        .headers
        .iter()
        .find(|(key, _value)| key.eq_ignore_ascii_case(name))
        .map(|(_key, value)| value.as_str())
}

fn generation_images(response: &ModelResponse) -> Value {
    Value::Array(
        response
            .data
            .iter()
            .map(|image| Value::String(image.b64_json.clone()))
            .collect(),
    )
}

fn edit_images(reply: &EditReply) -> Value {
    Value::Array(
        reply
            .response
            .data
            .iter()
            .flatten()
            .filter_map(|image| image.b64_json.as_deref())
            .map(|b64| Value::String(b64.to_owned()))
            .collect(),
    )
}

fn post_call(outcome: &Outcome) -> bool {
    outcome
        .state
        .model_call_details
        .contains_key("original_response")
}
