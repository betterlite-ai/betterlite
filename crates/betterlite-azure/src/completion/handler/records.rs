use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::post_call::PostCall;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::completion::arguments::CompletionCall;
use crate::completion::client::{ClientHandle, CompletionsTransport};

fn secret_json(value: Option<&SecretString>) -> Json {
    match value {
        Some(secret) => Json::Text(secret.expose_secret().to_owned()),
        None => Json::Null,
    }
}

fn text_or_null(value: Option<&str>) -> Json {
    match value {
        Some(text) => Json::Text(text.to_owned()),
        None => Json::Null,
    }
}

fn headers_json(headers: &SecretHeaders) -> IndexMap<String, Json> {
    headers
        .0
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

// pre_call(input=prompt, api_key=api_key, additional_args={"headers": {"api_key", "azure_ad_token"}, "api_version", "api_base", "complete_input_dict"})
pub(super) fn sync_pre_call<Client>(
    call: &CompletionCall<'_, Client>,
    input: &str,
    data: &Extra,
) -> PreCall {
    let mut headers = IndexMap::new();
    headers.insert("api_key".to_owned(), secret_json(call.api_key));
    headers.insert(
        "azure_ad_token".to_owned(),
        secret_json(call.azure_ad_token),
    );
    let mut passthrough = IndexMap::new();
    passthrough.insert("api_version".to_owned(), text_or_null(call.api_version));
    PreCall {
        input: Json::Text(input.to_owned()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: call.api_base.map(str::to_owned),
            headers: Some(headers),
            complete_input_dict: Some(RawBody::Object(data.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough,
        },
    }
}

// post_call(input=prompt, original_response=stringified_response, additional_args={"headers": headers, "api_version", "api_base"})
pub(super) fn sync_post_call<Client>(
    call: &CompletionCall<'_, Client>,
    input: &str,
    original: Json,
) -> PostCall {
    let mut passthrough = IndexMap::new();
    passthrough.insert("api_version".to_owned(), text_or_null(call.api_version));
    PostCall {
        original_response: original,
        input: Json::Text(input.to_owned()),
        additional_args: AdditionalArgs {
            api_base: call.api_base.map(str::to_owned),
            headers: call.headers.map(headers_json),
            complete_input_dict: None,
            request_str: None,
            openai_sdk: None,
            passthrough,
        },
    }
}

// the async and streaming calls log the client's own key, base url and the acompletion flag
pub(super) fn async_pre_call<Client: CompletionsTransport>(
    client: &ClientHandle<Client>,
    data: &Extra,
) -> PreCall {
    let authorization = match client.api_key.as_ref() {
        Some(secret) => format!("Bearer {}", secret.expose_secret()),
        None => "Bearer None".to_owned(),
    };
    let mut headers = IndexMap::new();
    headers.insert("Authorization".to_owned(), Json::Text(authorization));
    let mut passthrough = IndexMap::new();
    passthrough.insert("acompletion".to_owned(), Json::Bool(true));
    PreCall {
        input: data.get("prompt").cloned().unwrap_or(Json::Null),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(client.base_url.clone()),
            headers: Some(headers),
            complete_input_dict: Some(RawBody::Object(data.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough,
        },
    }
}
