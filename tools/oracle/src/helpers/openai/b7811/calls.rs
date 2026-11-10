use std::collections::BTreeMap;

use betterlite_openai::embedding::{
    EmbeddingRecord, EmbeddingResponse, EmbeddingUsage, EmbeddingVector,
};
use betterlite_openai::mongodb_vector_store::decode::decode_mongodb_search;
use betterlite_openai::mongodb_vector_store::environment::{
    MongoEnv, get_complete_url, validate_environment,
};
use betterlite_openai::mongodb_vector_store::error::{MongoDbError, error_for_status};
use betterlite_openai::mongodb_vector_store::prepare::prepare_search;
use betterlite_openai::mongodb_vector_store::search::search_request;
use betterlite_openai::vector_stores::Headers;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::{Call, DecodeCall, EnvironmentCall, PrepareCall, SearchCall, StatusCall, UrlCall};

pub(super) fn run_call(call: &Call) -> Value {
    match call {
        Call::ValidateEnvironment(input) => outcome(validate(input)),
        Call::CompleteUrl(input) => complete_url(input),
        Call::PrepareSearch(input) => outcome(prepare(input)),
        Call::SearchRequest(input) => outcome(search(input)),
        Call::DecodeSearch(input) => decode(input),
        Call::StatusError(input) => status(input),
    }
}

fn outcome<E>(result: Result<Value, E>) -> Value {
    result.map_or_else(
        |_error| json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    )
}

fn validate(input: &EnvironmentCall) -> Result<Value, MongoDbError> {
    let mut provided = Headers::new();
    for (name, value) in &input.headers {
        provided.insert(name.clone(), SecretString::from(value.as_str()));
    }
    let env = MongoEnv {
        api_key: input
            .env
            .get("MONGODB_SIDECAR_API_KEY")
            .map(|value| SecretString::from(value.as_str())),
    };
    let sent = validate_environment(provided, input.litellm_params.as_ref(), &env)?;
    let mut exposed = BTreeMap::new();
    for (name, value) in &sent {
        exposed.insert(name.as_str(), value.expose_secret());
    }
    Ok(json!(exposed))
}

fn complete_url(call: &UrlCall) -> Value {
    match get_complete_url(call.api_base.as_deref()) {
        Ok(url) => json!({"ok": true, "value": url}),
        Err(_error) => json!({"ok": false}),
    }
}

fn prepare(call: &PrepareCall) -> Result<Value, MongoDbError> {
    let prepared = prepare_search(
        &call.litellm_params,
        &call.optional_params,
        call.extra_body.as_ref(),
        &call.query,
    )?;
    Ok(json!(prepared))
}

fn search(call: &SearchCall) -> Result<Value, MongoDbError> {
    let prepared = prepare_search(
        &call.litellm_params,
        &call.optional_params,
        call.extra_body.as_ref(),
        &call.query,
    )?;
    let embedding = embedding_response(&call.embedding);
    let request = search_request(&prepared, &embedding, &call.vector_store_id, &call.api_base)?;
    Ok(json!(request))
}

fn embedding_response(values: &[EmbeddingVector]) -> EmbeddingResponse {
    EmbeddingResponse {
        object: "list".to_owned(),
        data: values
            .iter()
            .enumerate()
            .map(|(index, vector)| EmbeddingRecord {
                object: "embedding".to_owned(),
                embedding: vector.clone(),
                index: u32::try_from(index).unwrap_or(u32::MAX),
            })
            .collect(),
        model: "helper".to_owned(),
        usage: EmbeddingUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            total_tokens: 0,
        },
    }
}

fn decode(call: &DecodeCall) -> Value {
    match decode_mongodb_search(call.body.as_bytes()) {
        Ok(response) => json!({"ok": true, "value": response}),
        Err(_error) => json!({"ok": false}),
    }
}

// LiteLLM's exception classes carry these statuses; the case compares the mapping, not class or message
fn status(call: &StatusCall) -> Value {
    let error = error_for_status(call.message.clone(), call.status_code);
    let code: u16 = match error {
        MongoDbError::BadRequest { .. } => 400,
        MongoDbError::Authentication => 401,
        MongoDbError::Timeout { .. } => 408,
        MongoDbError::ServiceUnavailable { .. } => 503,
    };
    json!({"ok": true, "value": {"status": code}})
}
