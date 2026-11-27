use std::collections::BTreeMap;
use std::future::{Ready, ready};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::request_policy::{Params, RequestValue};
use betterlite_bedrock::s3_access::{S3AccessParams, TrustedValue, frozen_mapping};
use betterlite_bedrock::s3_listing::requests::{
    S3FileMethod, S3FilesRequest, S3SignedRequest, list_files_next_request, list_files_request,
    object_file_request, retrieve_file,
};
use betterlite_bedrock::s3_listing::responses::{
    S3RawResponse, S3ResponseContext, S3ResponseRequest, delete_file_response, list_files_response,
};
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};
use tokio::runtime::{Builder, Runtime};

use super::input::{DeleteResponseCall, FileCall, Input, ListingCall, Operation, WireValue};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

// the Python side patches botocore's clock and pins the environment pair to these values
const SIGNING_SECONDS: u64 = 1_704_067_200;
const ENV_ACCESS_KEY: &str = "AKIDEXAMPLE";
const ENV_KEY_VALUE: &str = "secret";

const LISTING_URL: &str = "https://s3.us-east-1.amazonaws.com/bucket/?list-type=2";
const DELETE_URL: &str =
    "https://s3.us-east-1.amazonaws.com/bucket/litellm-batch-outputs%2Fx.jsonl";

pub(super) fn run(input: &Input) -> HelperResult {
    match input.operation {
        Operation::ListRequest => list_requests(input),
        Operation::NextRequest => next_requests(input),
        Operation::ListResponse => list_responses(input),
        Operation::DeleteRequest => file_requests(&input.deletes, S3FileMethod::Delete),
        Operation::ContentRequest => file_requests(&input.contents, S3FileMethod::Content),
        Operation::DeleteResponse => delete_responses(input),
        Operation::Retrieve => retrieves(input),
    }
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread().build().or_value_error()
}

fn signing_now() -> Result<SystemTime, Box<HelperError>> {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(SIGNING_SECONDS))
        .ok_or("the signing time is out of range")
        .or_value_error()
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

fn trusted_value(value: &WireValue) -> TrustedValue {
    match value {
        WireValue::Text(text) => TrustedValue::Text(text.clone()),
        WireValue::Boolean(flag) => TrustedValue::Boolean(*flag),
        WireValue::Null(()) => TrustedValue::Null,
        WireValue::Other(_) => TrustedValue::Other,
    }
}

// the case snapshot is the trusted _litellm_internal_model_credentials; request buckets never authorize
fn access_params(trusted: Option<&[(String, WireValue)]>) -> S3AccessParams {
    let entries = trusted
        .unwrap_or_default()
        .iter()
        .map(|(name, value)| (name.clone(), trusted_value(value)));
    S3AccessParams {
        trusted: frozen_mapping(entries),
        bucket_name: None,
        output_bucket_name: None,
    }
}

fn region_inputs(environment: &BTreeMap<String, String>) -> RegionInputs<'_> {
    RegionInputs {
        environment_region_name: environment.get("AWS_REGION_NAME").map(String::as_str),
        environment_region: environment.get("AWS_REGION").map(String::as_str),
        session_region: None,
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
            access_key_id: SecretString::from(ENV_ACCESS_KEY.to_owned()),
            secret_access_key: SecretString::from(ENV_KEY_VALUE.to_owned()),
            session_token: None,
        },
    }
}

fn resolve_credentials(
    settings: &AwsSettings,
    _region: &str,
) -> Ready<Result<Credentials, CredentialError>> {
    ready(Ok(chain_identity(settings)))
}

fn query_map(query: &[(String, String)]) -> Value {
    let mut map = Map::new();
    for (name, value) in query {
        map.insert(name.clone(), Value::String(value.clone()));
    }
    Value::Object(map)
}

fn signed_value(signed: &S3SignedRequest, litellm_params: &Params) -> HelperResult {
    let query = query_map(&signed.query);
    Ok(json!({
        "failed": false,
        "url": signed.url.clone(),
        "query": query,
        "litellm_params": from_params(litellm_params)?,
    }))
}

fn next_value(signed: &S3SignedRequest, litellm_params: &Params) -> HelperResult {
    let next = json!({
        "url": signed.url.clone(),
        "query": query_map(&signed.query),
        "litellm_params": from_params(litellm_params)?,
    });
    Ok(json!({"failed": false, "next": next}))
}

fn list_requests(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    let now = signing_now()?;
    let mut results = Vec::new();
    for call in &input.requests {
        let optional = to_params(&call.optional_params);
        let mut litellm = to_params(&call.litellm_params);
        let access = access_params(call.trusted.as_deref());
        let request = S3FilesRequest {
            optional_params: &optional,
            litellm_params: &mut litellm,
            access: &access,
            environment: &call.environment,
            region_inputs: region_inputs(&call.environment),
            now,
        };
        let outcome = runtime.block_on(list_files_request(
            request,
            call.purpose.as_deref(),
            resolve_credentials,
        ));
        let entry = match outcome {
            Ok(signed) => signed_value(&signed, &litellm)?,
            Err(error) => json!({"failed": true, "status": error.status_code()}),
        };
        results.push(entry);
    }
    Ok(json!({"results": results}))
}

fn next_requests(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    let now = signing_now()?;
    let mut results = Vec::new();
    for call in &input.next {
        let optional = to_params(&call.optional_params);
        let mut litellm = to_params(&call.litellm_params);
        let access = access_params(call.trusted.as_deref());
        let request = S3FilesRequest {
            optional_params: &optional,
            litellm_params: &mut litellm,
            access: &access,
            environment: &call.environment,
            region_inputs: region_inputs(&call.environment),
            now,
        };
        let outcome = runtime.block_on(list_files_next_request(
            request,
            call.status,
            call.body.as_bytes(),
            resolve_credentials,
        ));
        let entry = match outcome {
            Ok(Some(signed)) => next_value(&signed, &litellm)?,
            Ok(None) => json!({"failed": false, "next": Value::Null}),
            Err(error) => json!({"failed": true, "status": error.status_code()}),
        };
        results.push(entry);
    }
    Ok(json!({"results": results}))
}

fn list_responses(input: &Input) -> HelperResult {
    let mut results = Vec::new();
    for call in &input.listings {
        results.push(listing_value(call)?);
    }
    Ok(json!({"results": results}))
}

fn listing_value(call: &ListingCall) -> HelperResult {
    let litellm = to_params(&call.litellm_params);
    let access = access_params(call.trusted.as_deref());
    let raw = S3RawResponse {
        status: call.status,
        body: call.body.as_bytes(),
        headers: &call.headers,
        request: S3ResponseRequest {
            method: "GET",
            url: LISTING_URL,
        },
    };
    let context = S3ResponseContext {
        litellm_params: &litellm,
        access: &access,
        environment: &call.environment,
    };
    match list_files_response(&raw, &context) {
        Ok(files) => Ok(json!({
            "failed": false,
            "files": serde_json::to_value(files).or_value_error()?,
        })),
        Err(error) => Ok(json!({"failed": true, "status": error.status_code()})),
    }
}

fn file_requests(calls: &[FileCall], method: S3FileMethod) -> HelperResult {
    let runtime = runtime()?;
    let now = signing_now()?;
    let mut results = Vec::new();
    for call in calls {
        let optional = to_params(&call.optional_params);
        let mut litellm = to_params(&call.litellm_params);
        let access = access_params(call.trusted.as_deref());
        let request = S3FilesRequest {
            optional_params: &optional,
            litellm_params: &mut litellm,
            access: &access,
            environment: &call.environment,
            region_inputs: region_inputs(&call.environment),
            now,
        };
        let outcome = runtime.block_on(object_file_request(
            request,
            method,
            call.file_id.as_deref(),
            resolve_credentials,
        ));
        let entry = match outcome {
            Ok(signed) => signed_value(&signed, &litellm)?,
            Err(error) => json!({"failed": true, "status": error.status_code()}),
        };
        results.push(entry);
    }
    Ok(json!({"results": results}))
}

fn delete_responses(input: &Input) -> HelperResult {
    let mut results = Vec::new();
    for call in &input.delete_responses {
        results.push(delete_value(call)?);
    }
    Ok(json!({"results": results}))
}

fn delete_value(call: &DeleteResponseCall) -> HelperResult {
    let additional = call.additional_args.as_ref().map(to_params);
    let raw = S3RawResponse {
        status: call.status,
        body: call.body.as_bytes(),
        headers: &call.headers,
        request: S3ResponseRequest {
            method: "DELETE",
            url: DELETE_URL,
        },
    };
    match delete_file_response(&raw, additional.as_ref()) {
        Ok(deleted) => Ok(json!({
            "failed": false,
            "file": serde_json::to_value(deleted).or_value_error()?,
        })),
        Err(error) => Ok(json!({"failed": true, "status": error.status_code()})),
    }
}

fn retrieves(input: &Input) -> HelperResult {
    let mut results = Vec::new();
    for _call in &input.retrieves {
        let status = retrieve_file().err().and_then(|error| error.status_code());
        results.push(json!({"failed": true, "status": status}));
        results.push(json!({"failed": true, "status": status}));
    }
    Ok(json!({"results": results}))
}
