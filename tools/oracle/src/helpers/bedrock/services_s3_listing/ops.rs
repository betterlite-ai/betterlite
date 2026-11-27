use betterlite_bedrock::request_policy::Params;
use betterlite_bedrock::s3_listing::requests::{
    S3FileMethod, S3SignedRequest, list_files_next_request, list_files_request,
    object_file_request, retrieve_file,
};
use betterlite_bedrock::s3_listing::responses::{
    S3RawResponse, S3ResponseContext, S3ResponseRequest, delete_file_response, list_files_response,
};
use serde_json::{Map, Value, json};

use super::input::{Call, Input, Operation};
use super::support::{
    access_params, from_params, request_of, resolve_credentials, runtime, signing_now, to_params,
};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

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
    let executor = runtime()?;
    let now = signing_now()?;
    let mut results = Vec::new();
    for call in &input.requests {
        let optional = to_params(&call.optional_params);
        let mut litellm = to_params(&call.litellm_params);
        let access = access_params(call.trusted.as_deref());
        let request = request_of(call, &mut litellm, &optional, &access, now);
        let outcome = executor.block_on(list_files_request(
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
    let executor = runtime()?;
    let now = signing_now()?;
    let mut results = Vec::new();
    for call in &input.next {
        let optional = to_params(&call.optional_params);
        let mut litellm = to_params(&call.litellm_params);
        let access = access_params(call.trusted.as_deref());
        let request = request_of(call, &mut litellm, &optional, &access, now);
        let outcome = executor.block_on(list_files_next_request(
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

fn file_requests(calls: &[Call], method: S3FileMethod) -> HelperResult {
    let executor = runtime()?;
    let now = signing_now()?;
    let mut results = Vec::new();
    for call in calls {
        let optional = to_params(&call.optional_params);
        let mut litellm = to_params(&call.litellm_params);
        let access = access_params(call.trusted.as_deref());
        let request = request_of(call, &mut litellm, &optional, &access, now);
        let outcome = executor.block_on(object_file_request(
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

fn list_responses(input: &Input) -> HelperResult {
    let mut results = Vec::new();
    for call in &input.listings {
        results.push(listing_value(call)?);
    }
    Ok(json!({"results": results}))
}

fn listing_value(call: &Call) -> HelperResult {
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

fn delete_responses(input: &Input) -> HelperResult {
    let mut results = Vec::new();
    for call in &input.delete_responses {
        results.push(delete_value(call)?);
    }
    Ok(json!({"results": results}))
}

fn delete_value(call: &Call) -> HelperResult {
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
