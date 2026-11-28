use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde_json::Number;

use crate::api::batch_enqueued_reserve::metadata::resolve_enqueued_scopes;
use crate::api::batch_enqueued_reserve::{BatchEnqueuedPrincipal, BatchEnqueuedTokenScope};
use crate::api::batch_limit_counters::records::BatchFileUsage;
use crate::api::batch_limit_descriptors::input::BatchLimitPrincipal;
use crate::api::request_destination::RequestData;

use super::BatchPrecallError;

pub(crate) const DEFAULT_LLM_PROVIDER: &str = "openai";
const INPUT_FILE_ID: &str = "input_file_id";
const CUSTOM_LLM_PROVIDER: &str = "custom_llm_provider";
const TOKEN_COUNT: &str = "_batch_token_count";
const REQUEST_COUNT: &str = "_batch_request_count";
const METADATA: &str = "metadata";
const REDIS_NAMESPACE: &str = "redis_namespace";

// resolve_batch_enqueued_token_scopes reads four typed fields off the caller; the wire record keeps them as JSON
pub(crate) fn enqueued_scopes(
    principal: &BatchLimitPrincipal,
) -> Result<Vec<BatchEnqueuedTokenScope>, BatchPrecallError> {
    let api_key = principal_text(principal, "api_key")?.map(SecretString::from);
    let team_id = principal_text(principal, "team_id")?.map(str::to_owned);
    let caller = BatchEnqueuedPrincipal {
        api_key,
        team_id,
        metadata: principal_object(principal, METADATA)?,
        team_metadata: principal_object(principal, "team_metadata")?,
    };
    Ok(resolve_enqueued_scopes(&caller))
}

pub(crate) fn principal_text<'value>(
    principal: &'value BatchLimitPrincipal,
    name: &'static str,
) -> Result<Option<&'value str>, BatchPrecallError> {
    match principal.0.get(name) {
        None | Some(JsonData::Null) => Ok(None),
        Some(JsonData::String(text)) => Ok(Some(text)),
        Some(_) => Err(BatchPrecallError::Field(name)),
    }
}

fn principal_object(
    principal: &BatchLimitPrincipal,
    name: &'static str,
) -> Result<Option<BTreeMap<String, JsonData>>, BatchPrecallError> {
    match principal.0.get(name) {
        None | Some(JsonData::Null) => Ok(None),
        Some(JsonData::Object(fields)) => Ok(Some(fields.clone())),
        Some(_) => Err(BatchPrecallError::Field(name)),
    }
}

// the source's `if not input_file_id`: nothing to read means the hook leaves the request alone
pub(crate) fn input_file_id(data: &RequestData) -> Result<Option<&str>, BatchPrecallError> {
    match data.get(INPUT_FILE_ID) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_) => Err(BatchPrecallError::Field(INPUT_FILE_ID)),
    }
}

pub(crate) fn custom_llm_provider(data: &RequestData) -> Result<&str, BatchPrecallError> {
    match data.get(CUSTOM_LLM_PROVIDER) {
        None | Some(Json::Null) => Ok(DEFAULT_LLM_PROVIDER),
        Some(Json::Text(text)) => Ok(text),
        Some(_) => Err(BatchPrecallError::Field(CUSTOM_LLM_PROVIDER)),
    }
}

// data["_batch_token_count"] and data["_batch_request_count"], the counts the route reads after the hook
pub(crate) fn insert_usage(data: &mut RequestData, usage: &BatchFileUsage) {
    data.insert(
        TOKEN_COUNT.to_owned(),
        Json::Number(Number::from(usage.total_tokens)),
    );
    data.insert(
        REQUEST_COUNT.to_owned(),
        Json::Number(Number::from(usage.request_count)),
    );
}

// data["metadata"]["redis_namespace"] = cache_key_name; a request without a metadata object is a swallowed failure
pub(crate) fn set_namespace(data: &mut RequestData, name: &str) -> Result<(), BatchPrecallError> {
    match data.get_mut(METADATA) {
        Some(Json::Object(fields)) => {
            fields.insert(REDIS_NAMESPACE.to_owned(), Json::Text(name.to_owned()));
            Ok(())
        }
        None | Some(Json::Null) => Err(BatchPrecallError::MetadataMissing),
        Some(_) => Err(BatchPrecallError::MetadataNotObject),
    }
}
