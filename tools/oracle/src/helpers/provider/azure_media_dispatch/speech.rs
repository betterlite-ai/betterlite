use betterlite_azure::endpoints::projection::{LiteLlmParams, SessionKind};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::azure_media_dispatch::AzureMediaResult;
use betterlite_provider::azure_media_dispatch::call::{
    AzureMediaOperation, NativeSsmlOperation, OpenAiSpeechOperation,
};
use betterlite_provider::azure_media_dispatch::test_support::{Reply, exchange};
use betterlite_provider::operation::OperationResult;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::case::Input;
use super::{first_request, header, outcome_error};
use crate::openai_media_dispatch::failure;

const SPEECH_REPLY: &[u8] = b"ID3";

pub(super) async fn openai_speech(input: &Input) -> HelperResult {
    let call = OpenAiSpeechOperation {
        model: input.model.clone(),
        input: input.input.clone(),
        voice: voice_name(input),
        optional_params: input.options.clone(),
        aspeech: false,
        api_key: Some(SecretString::from(input.api_key.clone())),
        api_base: Some(input.api_base.clone()),
        api_version: Some(input.api_version.clone()),
        params: LiteLlmParams::default(),
        provider: None,
        supplied: None,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
    };
    let outcome = exchange(
        AzureMediaOperation::OpenAiSpeech(Box::new(call)),
        vec![speech_reply()],
    )
    .await
    .map_err(|error| failure("RuntimeError", error))?;
    let Ok(OperationResult::AzureMedia(AzureMediaResult::OpenAiSpeech(content))) = &outcome.result
    else {
        return outcome_error(&outcome.result);
    };
    speech_view(
        &first_request(outcome.fixture.outbound())?,
        content.status,
        &content.body,
        "api-key",
    )
}

pub(super) async fn native_ssml(input: &Input) -> HelperResult {
    let call = NativeSsmlOperation {
        model: input.model.clone(),
        input: input.input.clone(),
        voice: input.voice.clone(),
        optional_params: input.options.clone(),
        litellm_params: IndexMap::new(),
        api_key: Some(SecretString::from(input.api_key.clone())),
        api_base: Some(input.api_base.clone()),
        timeout: None,
        extra_headers: None,
        aspeech: false,
    };
    let outcome = exchange(
        AzureMediaOperation::NativeSsml(Box::new(call)),
        vec![speech_reply()],
    )
    .await
    .map_err(|error| failure("RuntimeError", error))?;
    let Ok(OperationResult::AzureMedia(AzureMediaResult::NativeSsml(response))) = &outcome.result
    else {
        return outcome_error(&outcome.result);
    };
    speech_view(
        &first_request(outcome.fixture.outbound())?,
        response.content.status,
        &response.content.body,
        "ocp-apim-subscription-key",
    )
}

fn voice_name(input: &Input) -> String {
    match &input.voice {
        Some(Json::Text(name)) => name.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::from("alloy"),
    }
}

fn speech_reply() -> Reply {
    Reply {
        status: 200,
        headers: HeaderMap::new(),
        body: SPEECH_REPLY.to_vec(),
    }
}

fn speech_view(
    sent: &OutboundRequest,
    status: u16,
    content: &[u8],
    key_header: &str,
) -> HelperResult {
    let body = match &sent.body {
        RequestBody::Json(bytes) => {
            serde_json::from_slice(bytes).map_err(|error| failure("Decode", error))?
        }
        RequestBody::Bytes { data, .. } => {
            Value::String(String::from_utf8_lossy(data).into_owned())
        }
        RequestBody::Empty => Value::Null,
        RequestBody::Multipart { .. } => {
            return Err(failure("Fixture", "a speech request is not multipart"));
        }
    };
    Ok(json!({
        "method": sent.method.as_str(),
        "url": sent.url.as_str(),
        "api_key": header(&sent.headers, key_header),
        "authorization": header(&sent.headers, "authorization"),
        "accept": header(&sent.headers, "accept"),
        "output_format": header(&sent.headers, "x-microsoft-outputformat"),
        "body": body,
        "status": status,
        "content": String::from_utf8_lossy(content),
    }))
}
