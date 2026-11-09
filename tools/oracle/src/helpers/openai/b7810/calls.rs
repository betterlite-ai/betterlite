use std::collections::BTreeMap;

use betterlite_openai::milvus_vector_store::decode::decode_milvus_search;
use betterlite_openai::milvus_vector_store::error::MilvusError;
use betterlite_openai::milvus_vector_store::search::{
    Search, search_request as milvus_search_request,
};
use betterlite_openai::milvus_vector_store::{
    MilvusEnv, MilvusParams, create_request as milvus_create_request,
    create_response as milvus_create_response, get_auth_credentials, get_complete_url,
    map_openai_params, validate_environment,
};
use betterlite_openai::pg_vector_store::error::PgVectorError;
use betterlite_openai::pg_vector_store::{
    PgVectorEnv, PgVectorParams, get_complete_url as pg_complete_url,
    search_request as pg_search_request, validate_environment as pg_validate_environment,
};
use betterlite_openai::vector_stores::Headers;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::{
    Call, MilvusDecodeCall, MilvusMapCall, MilvusParamsCall, MilvusSearchCall, MilvusUrlCall,
    PgSearchCall, PgUrlCall,
};

pub(super) fn run_call(call: &Call) -> Value {
    match call {
        Call::MilvusAuthCredentials(input) => outcome(
            get_auth_credentials(&milvus_params(input)).map(|headers| headers_value(&headers)),
        ),
        Call::MilvusValidateEnvironment(input) => outcome(
            validate_environment(
                secret_headers(&input.headers),
                input.litellm_params.as_ref().map(milvus_params).as_ref(),
                &milvus_env(&input.env),
            )
            .map(|headers| headers_value(&headers)),
        ),
        Call::MilvusCompleteUrl(input) => milvus_url(input),
        Call::MilvusMapParams(input) => json!({"ok": true, "value": milvus_mapping(input)}),
        Call::MilvusSearchRequest(input) => outcome(milvus_search(input)),
        Call::MilvusDecodeSearch(input) => decode_outcome(input),
        Call::MilvusCreateRequest(input) => outcome(
            milvus_create_request(&input.optional_params, &input.api_base)
                .map(|request| json!(request)),
        ),
        Call::MilvusCreateResponse(input) => outcome(
            milvus_create_response(input.status_code, &input.headers)
                .map(|response| json!(response)),
        ),
        Call::PgVectorValidateEnvironment(input) => {
            let params = PgVectorParams {
                api_key: input.api_key.as_deref().map(SecretString::from),
            };
            outcome(
                pg_validate_environment(
                    secret_headers(&input.headers),
                    Some(&params),
                    &pg_env(&input.env),
                )
                .map(|headers| headers_value(&headers)),
            )
        }
        Call::PgVectorCompleteUrl(input) => pg_url(input),
        Call::PgVectorSearchRequest(input) => outcome(pg_search(input)),
    }
}

fn outcome<E>(result: Result<Value, E>) -> Value {
    result.map_or_else(
        |_error| json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    )
}

fn milvus_params(input: &MilvusParamsCall) -> MilvusParams {
    MilvusParams {
        api_key: input.api_key.as_deref().map(SecretString::from),
        milvus_db_name: input.milvus_db_name.clone(),
        milvus_partition_names: input.milvus_partition_names.clone(),
        milvus_text_field: input.milvus_text_field.clone(),
        litellm_embedding_model: input.litellm_embedding_model.clone(),
    }
}

fn milvus_env(env: &BTreeMap<String, String>) -> MilvusEnv {
    MilvusEnv {
        api_key: env
            .get("MILVUS_API_KEY")
            .map(|value| SecretString::from(value.as_str())),
        api_base: env.get("MILVUS_API_BASE").cloned(),
    }
}

fn pg_env(env: &BTreeMap<String, String>) -> PgVectorEnv {
    PgVectorEnv {
        api_key: env
            .get("PG_VECTOR_API_KEY")
            .map(|value| SecretString::from(value.as_str())),
        api_base: env.get("PG_VECTOR_API_BASE").cloned(),
    }
}

fn secret_headers(headers: &BTreeMap<String, String>) -> Headers {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect()
}

fn headers_value(headers: &Headers) -> Value {
    let exposed = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect::<BTreeMap<_, _>>();
    json!(exposed)
}

fn milvus_url(call: &MilvusUrlCall) -> Value {
    match get_complete_url(call.api_base.as_deref(), &milvus_env(&call.env)) {
        Ok(url) => json!({"ok": true, "value": url}),
        Err(_error) => json!({"ok": false}),
    }
}

fn milvus_mapping(call: &MilvusMapCall) -> Value {
    let mapped = map_openai_params(
        &call.non_default_params,
        call.optional_params.clone(),
        false,
    );
    json!(mapped)
}

fn milvus_search(call: &MilvusSearchCall) -> Result<Value, MilvusError> {
    let params = milvus_params(&call.litellm_params);
    milvus_search_request(Search {
        vector_store_id: &call.vector_store_id,
        query_text: &call.query_text,
        query_vector: &call.query_vector,
        optional_params: &call.optional_params,
        api_base: &call.api_base,
        litellm_params: &params,
    })
    .map(|request| json!(request))
}

fn decode_outcome(call: &MilvusDecodeCall) -> Value {
    let decoded = decode_milvus_search(
        call.status_code,
        &call.headers,
        call.body.as_bytes(),
        &call.query_text,
        &call.text_field,
    );
    match decoded {
        Ok(response) => json!({"ok": true, "value": response}),
        Err(MilvusError::Provider {
            status, headers, ..
        }) => json!({"ok": true, "value": {"status_code": status, "headers": headers}}),
        Err(_error) => json!({"ok": false}),
    }
}

fn pg_url(call: &PgUrlCall) -> Value {
    match pg_complete_url(call.api_base.as_deref(), &pg_env(&call.env)) {
        Ok(url) => json!({"ok": true, "value": url}),
        Err(_error) => json!({"ok": false}),
    }
}

fn pg_search(call: &PgSearchCall) -> Result<Value, PgVectorError> {
    pg_search_request(
        &call.vector_store_id,
        &call.query,
        &call.params,
        &call.api_base,
    )
    .map(|request| json!(request))
}
