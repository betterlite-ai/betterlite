use std::collections::BTreeMap;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::outbound::merge_query;
use betterlite_openai::api::batch_records::CreateBatchRequest;
use betterlite_openai::xai_batches::error::XaiBatchesError;
use betterlite_openai::xai_batches::jsonl::results_to_openai_jsonl;
use betterlite_openai::xai_batches::records::{XaiBatch, XaiBatchList, XaiBatchResult};
use betterlite_openai::xai_batches::transformation::{
    DEFAULT_BATCH_ENDPOINT, get_xai_auth_headers, results_params, to_create_batch_body,
    to_litellm_batch, to_openai_batch_list, xai_batches_url,
};
use http::header::{AUTHORIZATION, HeaderMap};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Create {
        #[serde(default)]
        env: BTreeMap<String, String>,
        xai_key: Option<String>,
        calls: Vec<CreateCall>,
    },
    Batch {
        calls: Vec<BatchCall>,
    },
    List {
        calls: Vec<ListCall>,
    },
    Jsonl {
        calls: Vec<JsonlCall>,
    },
    Params {
        #[serde(default)]
        env: BTreeMap<String, String>,
        xai_key: Option<String>,
        calls: Vec<ParamsCall>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCall {
    api_base: Option<String>,
    api_key: Option<String>,
    data: CreateBatchRequest,
    endpoint: Option<String>,
    batch: XaiBatch,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchCall {
    batch: XaiBatch,
    endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListCall {
    page: XaiBatchList,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JsonlCall {
    results: Vec<XaiBatchResult>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParamsCall {
    api_base: Option<String>,
    api_key: Option<String>,
    after: Option<String>,
    limit: Option<i64>,
}

fn settings(xai_key: Option<&str>, env: &BTreeMap<String, String>) -> Settings {
    Settings {
        xai_key: xai_key.map(SecretString::from),
        generic_api_key: None,
        environment_key: env
            .get("XAI_API_KEY")
            .filter(|value| !value.is_empty())
            .map(|value| SecretString::from(value.as_str())),
        api_base: env
            .get("XAI_API_BASE")
            .filter(|value| !value.is_empty())
            .cloned(),
    }
}

fn invalid(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn outcome(result: Result<Value, XaiBatchesError>) -> Value {
    match result {
        Ok(value) => value,
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

fn run_calls<T, F>(calls: &[T], result: F) -> Vec<Value>
where
    F: Fn(&T) -> Result<Value, XaiBatchesError>,
{
    let mut outcomes = Vec::with_capacity(calls.len());
    for call in calls {
        outcomes.push(outcome(result(call)));
    }
    outcomes
}

fn request_parts(
    api_base: Option<&str>,
    api_key: Option<&str>,
    settings: &Settings,
) -> Result<(String, HeaderMap), XaiBatchesError> {
    let url = xai_batches_url(api_base, None, "", settings)?;
    let key = api_key.map(SecretString::from);
    let headers = get_xai_auth_headers(HeaderMap::new(), key.as_ref(), settings)?;
    Ok((url, headers))
}

fn create_result(call: &CreateCall, settings: &Settings) -> Result<Value, XaiBatchesError> {
    let (url, headers) =
        request_parts(call.api_base.as_deref(), call.api_key.as_deref(), settings)?;
    let body = to_create_batch_body(&call.data)?;
    let endpoint = call.endpoint.as_deref().unwrap_or(DEFAULT_BATCH_ENDPOINT);
    let projected = serde_json::to_value(to_litellm_batch(&call.batch, endpoint))
        .map_err(|source| XaiBatchesError::Encode { source })?;
    Ok(json!({
        "ok": true,
        "url": url,
        "body": {"name": body.name, "input_file_id": body.input_file_id},
        "authorization": headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()),
        "batch": projected,
    }))
}

fn batch_result(call: &BatchCall) -> Result<Value, XaiBatchesError> {
    let endpoint = call.endpoint.as_deref().unwrap_or(DEFAULT_BATCH_ENDPOINT);
    let projected = serde_json::to_value(to_litellm_batch(&call.batch, endpoint))
        .map_err(|source| XaiBatchesError::Encode { source })?;
    Ok(json!({"ok": true, "batch": projected}))
}

fn list_result(call: &ListCall) -> Result<Value, XaiBatchesError> {
    let projected = serde_json::to_value(to_openai_batch_list(&call.page))
        .map_err(|source| XaiBatchesError::Encode { source })?;
    Ok(json!({"ok": true, "page": projected}))
}

fn jsonl_result(call: &JsonlCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let jsonl =
        results_to_openai_jsonl(&call.results).map_err(|error| invalid(error.to_string()))?;
    let text = String::from_utf8(jsonl).map_err(|error| invalid(error.to_string()))?;
    Ok(json!({"ok": true, "jsonl": text}))
}

fn params_result(call: &ParamsCall, settings: &Settings) -> Result<Value, XaiBatchesError> {
    let (base, _) = request_parts(call.api_base.as_deref(), call.api_key.as_deref(), settings)?;
    let mut url = Url::parse(&base).map_err(|source| XaiBatchesError::BaseUrl { source })?;
    merge_query(&mut url, &results_params(call.after.as_deref(), call.limit));
    Ok(json!({"ok": true, "url": url.to_string()}))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Create {
                env,
                xai_key,
                calls,
            } => {
                let settings = settings(xai_key.as_deref(), env);
                Ok(json!({"calls": run_calls(calls, |call| create_result(call, &settings))}))
            }
            Input::Batch { calls } => Ok(json!({"calls": run_calls(calls, batch_result)})),
            Input::List { calls } => Ok(json!({"calls": run_calls(calls, list_result)})),
            Input::Jsonl { calls } => {
                let outcomes = calls
                    .iter()
                    .map(jsonl_result)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({"calls": outcomes}))
            }
            Input::Params {
                env,
                xai_key,
                calls,
            } => {
                let settings = settings(xai_key.as_deref(), env);
                Ok(json!({"calls": run_calls(calls, |call| params_result(call, &settings))}))
            }
        }
    }
}

mod tests;
