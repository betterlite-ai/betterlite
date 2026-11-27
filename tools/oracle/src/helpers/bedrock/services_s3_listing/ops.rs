use betterlite_bedrock::request_policy::Params;
use betterlite_bedrock::s3_listing::requests::{
    S3FileMethod, S3SignedRequest, list_files_next_request, list_files_request,
    object_file_request, retrieve_file,
};
use betterlite_bedrock::s3_listing::responses::{
    S3RawResponse, S3ResponseContext, S3ResponseRequest, delete_file_response, list_files_response,
};
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::input::{Call, Input, Operation};
use super::support::{
    access_params, from_params, request_of, resolve_credentials, runtime, signing_now, to_params,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

const LISTING_URL: &str = "https://s3.us-east-1.amazonaws.com/bucket/?list-type=2";
const DELETE_URL: &str =
    "https://s3.us-east-1.amazonaws.com/bucket/litellm-batch-outputs%2Fx.jsonl";

#[derive(Debug, Clone, Copy)]
enum RequestKind {
    List,
    Next,
    File(S3FileMethod),
}

#[derive(Debug, Clone, Copy)]
enum ResponseKind {
    List,
    Delete,
}

#[derive(Serialize)]
struct SignedFields {
    url: String,
    query: Value,
    litellm_params: Value,
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input.operation {
        Operation::ListRequest => signed_requests(&input.requests, RequestKind::List),
        Operation::NextRequest => signed_requests(&input.next, RequestKind::Next),
        Operation::ListResponse => list_responses(input),
        Operation::DeleteRequest => {
            signed_requests(&input.deletes, RequestKind::File(S3FileMethod::Delete))
        }
        Operation::ContentRequest => {
            signed_requests(&input.contents, RequestKind::File(S3FileMethod::Content))
        }
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

fn signed_fields(
    signed: &S3SignedRequest,
    litellm_params: &Params,
) -> Result<SignedFields, Box<HelperError>> {
    Ok(SignedFields {
        url: signed.url.clone(),
        query: query_map(&signed.query),
        litellm_params: from_params(litellm_params)?,
    })
}

// the list and delete transforms answer (url, query), while the next-page transform nests both
fn success_entry(signed: &S3SignedRequest, litellm_params: &Params, nested: bool) -> HelperResult {
    let fields = signed_fields(signed, litellm_params)?;
    if nested {
        let next = serde_json::to_value(fields).or_value_error()?;
        return Ok(json!({"failed": false, "next": next}));
    }
    Ok(json!({
        "failed": false,
        "url": fields.url,
        "query": fields.query,
        "litellm_params": fields.litellm_params,
    }))
}

fn signed_requests(calls: &[Call], kind: RequestKind) -> HelperResult {
    let executor = runtime()?;
    let now = signing_now()?;
    let mut results = Vec::new();
    for call in calls {
        let optional = to_params(&call.optional_params);
        let mut litellm = to_params(&call.litellm_params);
        let access = access_params(call.trusted.as_deref());
        let request = request_of(call, &mut litellm, &optional, &access, now);
        let outcome = match kind {
            RequestKind::List => executor
                .block_on(list_files_request(
                    request,
                    call.purpose.as_deref(),
                    resolve_credentials,
                ))
                .map(Some),
            RequestKind::Next => executor.block_on(list_files_next_request(
                request,
                call.status,
                call.body.as_bytes(),
                resolve_credentials,
            )),
            RequestKind::File(method) => executor
                .block_on(object_file_request(
                    request,
                    method,
                    call.file_id.as_deref(),
                    resolve_credentials,
                ))
                .map(Some),
        };
        let entry = match outcome {
            Ok(Some(signed)) => {
                success_entry(&signed, &litellm, matches!(kind, RequestKind::Next))?
            }
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
        results.push(response_value(call, ResponseKind::List)?);
    }
    Ok(json!({"results": results}))
}

fn delete_responses(input: &Input) -> HelperResult {
    let mut results = Vec::new();
    for call in &input.delete_responses {
        results.push(response_value(call, ResponseKind::Delete)?);
    }
    Ok(json!({"results": results}))
}

fn response_value(call: &Call, kind: ResponseKind) -> HelperResult {
    let (method, url) = match kind {
        ResponseKind::List => ("GET", LISTING_URL),
        ResponseKind::Delete => ("DELETE", DELETE_URL),
    };
    let raw = S3RawResponse {
        status: call.status,
        body: call.body.as_bytes(),
        headers: &call.headers,
        request: S3ResponseRequest { method, url },
    };
    match kind {
        ResponseKind::List => {
            let litellm = to_params(&call.litellm_params);
            let access = access_params(call.trusted.as_deref());
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
        ResponseKind::Delete => {
            let additional = call.additional_args.as_ref().map(to_params);
            match delete_file_response(&raw, additional.as_ref()) {
                Ok(deleted) => Ok(json!({
                    "failed": false,
                    "file": serde_json::to_value(deleted).or_value_error()?,
                })),
                Err(error) => Ok(json!({"failed": true, "status": error.status_code()})),
            }
        }
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
