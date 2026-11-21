use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::post_call::PostCall;
use betterlite_models::decisions;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_stream_errors::records::PreCallRecord;

use super::arguments::ChatCall;
use super::client::ClientHandle;

// the logging records hold betterlite-models' JSON, the request body holds betterlite-prompts' JSON
pub(super) fn logged(value: &Json) -> decisions::Json {
    match value {
        Json::Null => decisions::Json::Null,
        Json::Bool(flag) => decisions::Json::Bool(*flag),
        Json::Number(number) => decisions::Json::Number(number.clone()),
        Json::Text(text) => decisions::Json::Text(text.clone()),
        Json::List(items) => decisions::Json::List(items.iter().map(logged).collect()),
        Json::Object(fields) => decisions::Json::Object(logged_object(fields)),
    }
}

pub(super) fn logged_object(object: &Object) -> IndexMap<String, decisions::Json> {
    object
        .iter()
        .map(|(name, value)| (name.clone(), logged(value)))
        .collect()
}

fn secret_json(secret: Option<&SecretString>) -> decisions::Json {
    secret.map_or(decisions::Json::Null, |value| {
        decisions::Json::Text(value.expose_secret().to_owned())
    })
}

fn headers_object(headers: &SecretHeaders) -> IndexMap<String, decisions::Json> {
    headers
        .0
        .iter()
        .map(|(name, value)| (name.clone(), decisions::Json::Text(value.clone())))
        .collect()
}

fn token_headers<Client>(call: &ChatCall<'_, Client>) -> IndexMap<String, decisions::Json> {
    IndexMap::from([
        ("api_key".to_owned(), secret_json(call.api_key)),
        (
            "azure_ad_token".to_owned(),
            secret_json(call.azure_ad_token),
        ),
    ])
}

fn version_args<Client>(call: &ChatCall<'_, Client>) -> IndexMap<String, decisions::Json> {
    IndexMap::from([(
        "api_version".to_owned(),
        decisions::Json::Text(call.api_version.to_owned()),
    )])
}

const fn extra_args(
    api_base: Option<String>,
    headers: Option<IndexMap<String, decisions::Json>>,
    complete_input_dict: Option<RawBody>,
    openai_sdk: Option<bool>,
    passthrough: IndexMap<String, decisions::Json>,
) -> AdditionalArgs {
    AdditionalArgs {
        api_base,
        headers,
        complete_input_dict,
        request_str: None,
        openai_sdk,
        passthrough,
    }
}

// pre_call(input=messages, api_key=api_key, additional_args={"headers", "api_version", "api_base", "complete_input_dict", "openai_sdk"})
pub(super) fn sync_pre_call<Client>(
    call: &ChatCall<'_, Client>,
    input: decisions::Json,
    data: &Object,
) -> PreCallRecord {
    let mut additional_args = extra_args(
        Some(call.api_base.to_owned()),
        Some(token_headers(call)),
        None,
        None,
        version_args(call),
    );
    additional_args.complete_input_dict = Some(RawBody::Object(logged_object(data)));
    additional_args.openai_sdk = Some(true);
    PreCallRecord {
        input,
        api_key: call.api_key.cloned(),
        additional_args,
    }
}

// post_call(input=messages, api_key=api_key, original_response=stringified_response, additional_args={"headers", "api_version", "api_base"})
pub(super) fn sync_post_call<Client>(
    call: &ChatCall<'_, Client>,
    input: decisions::Json,
    original: decisions::Json,
    headers: &SecretHeaders,
) -> PostCall {
    let mut additional_args = extra_args(
        Some(call.api_base.to_owned()),
        None,
        None,
        None,
        version_args(call),
    );
    additional_args.headers = Some(headers_object(headers));
    PostCall {
        original_response: original,
        input,
        additional_args,
    }
}

// pre_call(input=data["messages"], api_key=azure_client.api_key, additional_args={"headers", "api_base", "acompletion", "complete_input_dict", "openai_sdk"})
pub(super) fn async_pre_call<Client>(
    call: &ChatCall<'_, Client>,
    client: &ClientHandle<Client>,
    data: &Object,
) -> PreCallRecord {
    PreCallRecord {
        input: data.get("messages").map_or(decisions::Json::Null, logged),
        api_key: client.api_key.clone(),
        additional_args: extra_args(
            Some(call.api_base.to_owned()),
            Some(token_headers(call)),
            Some(RawBody::Object(logged_object(data))),
            Some(true),
            IndexMap::from([("acompletion".to_owned(), decisions::Json::Bool(true))]),
        ),
    }
}

// post_call(input=data["messages"], original_response=stringified_response, additional_args={"complete_input_dict"})
pub(super) fn async_post_call(data: &Object, original: decisions::Json) -> PostCall {
    PostCall {
        original_response: original,
        input: data.get("messages").map_or(decisions::Json::Null, logged),
        additional_args: extra_args(
            None,
            None,
            Some(RawBody::Object(logged_object(data))),
            None,
            IndexMap::new(),
        ),
    }
}
