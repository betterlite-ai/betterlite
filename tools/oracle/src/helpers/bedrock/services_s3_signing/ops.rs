use std::collections::BTreeMap;
use std::future::ready;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::bedrock_settings::ErrorHeaders;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::request_policy::{Params, RequestValue};
use betterlite_bedrock::s3_signing::destination::{UploadTarget, get_complete_file_url};
use betterlite_bedrock::s3_signing::file_object::{
    get_error_class, transform_create_file_response,
};
use betterlite_bedrock::s3_signing::naming::get_object_name;
use betterlite_bedrock::s3_signing::signer::{S3PutRequest, sign_s3_request};
use betterlite_bedrock::s3_signing::{
    CUSTOM_LLM_PROVIDER, FILE_UPLOAD_HTTP_METHOD, SUPPORTED_OPENAI_PARAMS,
    UPLOAD_CONTENT_LENGTH_PARAM, UPLOAD_URL_PARAM, validate_environment,
};
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_prompts::media_metadata::{ExtractedFileData, FileBody, FileData};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::{RawValue, to_raw_value};
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;
use uuid::Uuid;

use super::input::{ErrorCall, Handoff, Input, ObjectFile, Operation, Signing, Upload};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

// the Python side patches fastuuid.uuid4 and botocore's clock to these values
const OBJECT_ID: Uuid = Uuid::from_u128(0x1111_1111_2222_4333_8444_5555_5555_5555);
const SIGNING_SECONDS: u64 = 1_704_067_200;

pub(super) fn run(input: &Input) -> HelperResult {
    match input.operation {
        Operation::ObjectName => {
            let names = input
                .files
                .iter()
                .map(object_name)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!({"results": names}))
        }
        Operation::UrlConversion => {
            let files = input
                .urls
                .iter()
                .map(String::as_str)
                .map(converted)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!({"results": files}))
        }
        Operation::Sign => signings(&input.signings),
        Operation::FileResponse => file_responses(input),
        Operation::Destination => {
            Ok(json!({"results": input.uploads.iter().map(destination).collect::<Vec<_>>()}))
        }
        Operation::Contract => contract(input),
    }
}

fn file_responses(input: &Input) -> HelperResult {
    let mut files = Vec::new();
    for response in &input.responses {
        files.push(file_reply(
            &response.litellm_params,
            &response.headers,
            response.now,
        )?);
    }
    let errors: Result<Vec<_>, _> = input.errors.iter().map(error_class).collect();
    Ok(json!({"results": files, "errors": errors?}))
}

fn to_params(map: &Map<String, Value>) -> Params {
    let mut params = Params::new();
    for (name, value) in map {
        let entry = match value {
            Value::String(text) => RequestValue::Text(text.clone()),
            Value::Object(object) => RequestValue::Mapping(to_params(object)),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => {
                RequestValue::Raw(value.to_string())
            }
        };
        params.insert(name.clone(), entry);
    }
    params
}

fn from_params(params: &Params) -> HelperResult {
    let mut map = Map::new();
    for (name, value) in params {
        let entry = match value {
            RequestValue::Text(text) => Value::String(text.clone()),
            RequestValue::Secret(secret) => Value::String(secret.expose_secret().to_owned()),
            RequestValue::Mapping(mapping) => from_params(mapping)?,
            RequestValue::Raw(raw) => serde_json::from_str(raw).or_value_error()?,
        };
        map.insert(name.clone(), entry);
    }
    Ok(Value::Object(map))
}

fn object_name(file: &ObjectFile) -> HelperResult {
    let content = match &file.content_hex {
        Some(encoded) => hex::decode(encoded).or_value_error()?,
        None => file.content.as_bytes().to_vec(),
    };
    let extracted = ExtractedFileData {
        filename: file.filename.clone(),
        content,
        content_type: "application/octet-stream".to_owned(),
        headers: BTreeMap::new(),
    };
    Ok(match get_object_name(&extracted, file.purpose, OBJECT_ID) {
        Ok(name) => json!({"failed": false, "name": name}),
        Err(_error) => json!({"failed": true}),
    })
}

fn file_reply(params: &Map<String, Value>, headers: &[(String, String)], now: i64) -> HelperResult {
    match transform_create_file_response(&to_params(params), headers, now) {
        Ok(file) => {
            Ok(json!({"failed": false, "file": serde_json::to_value(file).or_value_error()?}))
        }
        Err(_error) => Ok(json!({"failed": true})),
    }
}

// the Python side reads the URL back through transform_create_file_response, so the same call runs here
fn converted(url: &str) -> HelperResult {
    let mut params = Map::new();
    params.insert(UPLOAD_URL_PARAM.to_owned(), Value::String(url.to_owned()));
    params.insert(UPLOAD_CONTENT_LENGTH_PARAM.to_owned(), json!(0_i32));
    file_reply(&params, &[], 0)
}

fn error_class(call: &ErrorCall) -> HelperResult {
    let headers = ErrorHeaders::Http(call.headers.clone());
    let error =
        get_error_class(call.status_code, call.message.clone(), headers).or_value_error()?;
    Ok(json!({"status_code": error.error.status_code, "message": error.error.message}))
}

fn destination(upload: &Upload) -> Value {
    let litellm_params = to_params(&upload.litellm_params);
    let optional_params = to_params(&upload.optional_params);
    let file = upload.file.as_ref().map(|file| FileData::Tuple2 {
        filename: file.filename.clone(),
        content: FileBody::Bytes(file.content.as_bytes().to_vec()),
    });
    let target = UploadTarget {
        model: &upload.model,
        litellm_params: &litellm_params,
        optional_params: &optional_params,
        file: file.as_ref(),
        purpose: upload.purpose,
        environment: &upload.environment,
        region_inputs: RegionInputs {
            environment_region_name: upload
                .environment
                .get("AWS_REGION_NAME")
                .map(String::as_str),
            environment_region: upload.environment.get("AWS_REGION").map(String::as_str),
            session_region: None,
        },
        id: OBJECT_ID,
    };
    match get_complete_file_url(&target) {
        Ok(url) => json!({"failed": false, "url": url}),
        Err(_error) => json!({"failed": true}),
    }
}

// boto3's chain as the Python side sets it up: request keys sign as given, otherwise the environment pair
fn chain_identity(settings: &AwsSettings) -> Credentials {
    match (&settings.access_key_id, &settings.secret_access_key) {
        (Some(access_key_id), Some(secret_access_key)) => Credentials {
            access_key_id: access_key_id.clone(),
            secret_access_key: secret_access_key.clone(),
            session_token: settings.session_token.clone(),
        },
        (Some(_) | None, _) => Credentials {
            access_key_id: SecretString::from("AKIDEXAMPLE".to_owned()),
            secret_access_key: SecretString::from("secret".to_owned()),
            session_token: None,
        },
    }
}

fn signings(signings: &[Signing]) -> HelperResult {
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let now = UNIX_EPOCH
        .checked_add(Duration::from_secs(SIGNING_SECONDS))
        .ok_or("the signing time is out of range")
        .or_value_error()?;
    let mut results = Vec::new();
    for signing in signings {
        let mut optional_params: BTreeMap<String, Box<RawValue>> = BTreeMap::new();
        for (name, value) in &signing.optional_params {
            optional_params.insert(name.clone(), to_raw_value(value).or_value_error()?);
        }
        let key = signing.encryption_key_id.clone().map(SecretString::from);
        let request = S3PutRequest {
            content: &signing.content,
            api_base: &signing.api_base,
            optional_params: &optional_params,
            encryption_key_id: key.as_ref(),
            region_inputs: RegionInputs::default(),
            now,
        };
        let resolve = |settings: &AwsSettings, _region: &str| {
            ready(Ok::<Credentials, CredentialError>(chain_identity(settings)))
        };
        results.push(match runtime.block_on(sign_s3_request(request, resolve)) {
            Ok(signed) => json!({
                "failed": false,
                "headers": signed.headers,
                "body": String::from_utf8(signed.body).or_value_error()?,
            }),
            Err(_error) => json!({"failed": true}),
        });
    }
    Ok(json!({"results": results}))
}

fn handoff(call: &Handoff) -> HelperResult {
    let headers = to_params(&call.headers);
    let mut litellm_params = to_params(&call.litellm_params);
    let merged = validate_environment(&headers, &mut litellm_params);
    Ok(json!({"headers": from_params(&merged)?, "litellm_params": from_params(&litellm_params)?}))
}

fn contract(input: &Input) -> HelperResult {
    let handoffs = input
        .handoffs
        .iter()
        .map(handoff)
        .collect::<Result<Vec<_>, _>>()?;
    // map_openai_params returns the optional parameters it was given, so betterlite has no call for it
    Ok(json!({
        "custom_llm_provider": CUSTOM_LLM_PROVIDER,
        "file_upload_http_method": FILE_UPLOAD_HTTP_METHOD,
        "supported_openai_params": SUPPORTED_OPENAI_PARAMS,
        "mapped": input.optional_params,
        "handoffs": handoffs,
    }))
}
