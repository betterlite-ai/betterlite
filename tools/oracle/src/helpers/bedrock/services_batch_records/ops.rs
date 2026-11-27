use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::batch_records::jsonl::{BatchRecordContext, transform_openai_jsonl};
use betterlite_bedrock::batch_records::record::{BedrockBatchRecord, decode_batch_input_line};
use betterlite_bedrock::batch_records::upload::{
    CreateFileRequest as UploadRequest, FileUploadHandoff, transform_create_file_request,
};
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::request_policy::{Params as RequestParams, RequestValue};
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use uuid::Uuid;

use super::families::Families;
use super::fixture::{CaseLookup, Sources, epoch, helper_error, secrets};
use super::input::{CreateFileRequest, Input, TransformRecords};

// The families run synchronous operations and a ready credential seam, so every poll has its answer.
fn block_on_ready<F>(future: F) -> F::Output
where
    F: Future,
{
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn transform_records(call: &TransformRecords) -> HelperResult {
    let sources = Sources::new()?;
    let resolution = sources.resolution();
    let families = Families::new(&call.model_cost)?;
    let lookup = CaseLookup {
        model_cost: &call.model_cost,
    };
    let mut records = Vec::with_capacity(call.records.len());
    for value in &call.records {
        let bytes = serde_json::to_vec(value).or_value_error()?;
        let record = decode_batch_input_line(&bytes)
            .map_err(|error| helper_error(error, None))?
            .ok_or_else(|| helper_error("a record line is blank", None))?;
        records.push(record);
    }
    let context = BatchRecordContext {
        aliases: &call.model_alias_map,
        families: &families,
        lookup: &lookup,
        resolution: &resolution,
    };
    let transformed = block_on_ready(transform_openai_jsonl(
        &records,
        &call.target_model,
        &context,
    ))
    .map_err(|error| helper_error(error, None))?;
    let answer: Vec<Json> = transformed
        .iter()
        .map(BedrockBatchRecord::to_json)
        .collect();
    Ok(json!({ "records": answer }))
}

fn request_value(value: &Value) -> RequestValue {
    if let Value::String(text) = value {
        return RequestValue::Text(text.clone());
    }
    let Value::Object(fields) = value else {
        return RequestValue::Raw(value.to_string());
    };
    let mut mapped = RequestParams::new();
    for (name, entry) in fields {
        mapped.insert(name.clone(), request_value(entry));
    }
    RequestValue::Mapping(mapped)
}

fn request_params(value: &Value) -> Result<RequestParams, Box<HelperError>> {
    match value {
        Value::Object(fields) => Ok(fields
            .iter()
            .map(|(name, entry)| (name.clone(), request_value(entry)))
            .collect()),
        Value::Null => Ok(RequestParams::new()),
        Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Err(helper_error("the parameters must be an object", None))
        }
    }
}

fn request_json(value: &RequestValue) -> Value {
    match value {
        RequestValue::Text(text) => Value::String(text.clone()),
        RequestValue::Secret(secret) => Value::String(secret.expose_secret().to_owned()),
        RequestValue::Mapping(fields) => params_json(fields),
        RequestValue::Raw(text) => serde_json::from_str(text).unwrap_or(Value::Null),
    }
}

fn params_json(params: &RequestParams) -> Value {
    Value::Object(
        params
            .iter()
            .map(|(name, value)| (name.clone(), request_json(value)))
            .collect(),
    )
}

fn headers_json(headers: &[(String, String)]) -> Value {
    Value::Object(
        headers
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect(),
    )
}

const DEFAULT_ACCESS_KEY: &str = "AKIDEXAMPLE";
const DEFAULT_SECRET_KEY: &str = "secret";

// The Python side sets the environment pair to these defaults before boto3's chain reads it.
fn held_credentials(environment: &BTreeMap<String, String>) -> Credentials {
    let access_key_id = environment
        .get("AWS_ACCESS_KEY_ID")
        .map_or(DEFAULT_ACCESS_KEY, String::as_str);
    let secret_access_key = environment
        .get("AWS_SECRET_ACCESS_KEY")
        .map_or(DEFAULT_SECRET_KEY, String::as_str);
    Credentials {
        access_key_id: SecretString::from(access_key_id.to_owned()),
        secret_access_key: SecretString::from(secret_access_key.to_owned()),
        session_token: environment
            .get("AWS_SESSION_TOKEN")
            .map(|token| SecretString::from(token.clone())),
    }
}

fn upload_file(call: &CreateFileRequest) -> FileData {
    FileData::Tuple3 {
        filename: call.file.filename.clone(),
        content: FileBody::Bytes(call.file.content.as_bytes().to_vec()),
        content_type: call.file.content_type.clone(),
    }
}

fn upload_answer(handoff: &FileUploadHandoff, litellm: &RequestParams) -> Value {
    json!({
        "failed": false,
        "method": handoff.method,
        "url": handoff.url,
        "headers": headers_json(&handoff.headers),
        "data": handoff.data,
        "litellm_params": params_json(litellm),
    })
}

fn sign_upload(
    call: &CreateFileRequest,
    context: &BatchRecordContext<'_, CaseLookup<'_>>,
    file: &FileData,
) -> Result<(Option<FileUploadHandoff>, RequestParams), Box<HelperError>> {
    let optional = request_params(&call.optional_params)?;
    let mut litellm = request_params(&call.litellm_params)?;
    let environment = call.environment.clone();
    let held = held_credentials(&environment);
    let secret_lookup = secrets();
    let mut request = UploadRequest {
        model: &call.model,
        file: Some(file),
        purpose: call.purpose,
        optional_params: &optional,
        litellm_params: &mut litellm,
        environment: &environment,
        secrets: &secret_lookup,
        region_inputs: RegionInputs {
            environment_region_name: environment.get("AWS_REGION_NAME").map(String::as_str),
            environment_region: environment.get("AWS_REGION").map(String::as_str),
            session_region: None,
        },
        id: Uuid::parse_str(&call.uuid).or_value_error()?,
        now: epoch(call.now)?,
    };
    let outcome = block_on_ready(transform_create_file_request(
        &mut request,
        context,
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(held.clone())),
    ));
    Ok((outcome.ok(), litellm))
}

fn upload(call: &CreateFileRequest) -> HelperResult {
    let sources = Sources::new()?;
    let resolution = sources.resolution();
    let families = Families::new(&call.model_cost)?;
    let lookup = CaseLookup {
        model_cost: &call.model_cost,
    };
    let context = BatchRecordContext {
        aliases: &call.model_alias_map,
        families: &families,
        lookup: &lookup,
        resolution: &resolution,
    };
    let file = upload_file(call);
    let (signed, litellm) = sign_upload(call, &context, &file)?;
    match signed {
        Some(handoff) => Ok(upload_answer(&handoff, &litellm)),
        None => Ok(json!({ "failed": true })),
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::TransformRecords(call) => transform_records(call),
        Input::CreateFileRequest(call) => upload(call),
    }
}
