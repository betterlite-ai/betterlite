use std::time::SystemTime;

use betterlite_openai::api::speech_provider_contract::{SpeechVoice, TextToSpeechRequestData};
use betterlite_openai::openai_base_config::Object;
use serde_json::json;

use super::body::{request_body, serialize};
use super::credentials::{self, CredentialResolver};
use super::endpoint::{DEFAULT_REGION, complete_url};
use super::error::SpeechError;
use super::region::{self, RegionEnvironment};
use super::signing::{SignablePost, sign_post};
use super::values::text_field;
use super::voice::dispatch_voice;

pub struct Dispatch<'call> {
    pub voice: Option<&'call SpeechVoice>,
    pub optional_params: Object,
    pub litellm_params: Object,
    pub kwargs: &'call Object,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call str>,
    pub environment: RegionEnvironment<'call>,
}

pub struct Dispatched {
    pub voice: Option<String>,
    pub optional_params: Object,
    pub litellm_params: Object,
}

// dispatch_text_to_speech: the region decides, then the effective litellm_params replace the given names
pub fn dispatch(call: Dispatch<'_>) -> Result<Dispatched, SpeechError> {
    let kwargs_region = text_field(call.kwargs, "aws_region_name")?;
    let optional_region = text_field(&call.optional_params, "aws_region_name")?;
    let resolved = region::dispatch_region(kwargs_region, optional_region, call.environment)?;
    let mut litellm_params = call.litellm_params;
    litellm_params.insert(String::from("aws_region_name"), json!(resolved));
    litellm_params.insert(
        String::from("api_base"),
        call.api_base
            .map_or_else(|| json!(null), |base| json!(base)),
    );
    litellm_params.insert(
        String::from("api_key"),
        call.api_key.map_or_else(|| json!(null), |key| json!(key)),
    );
    Ok(Dispatched {
        voice: dispatch_voice(call.voice),
        optional_params: call.optional_params,
        litellm_params,
    })
}

// get_complete_url reads both names off litellm_params, and a null base means the partition host
pub fn configured_url(litellm_params: &Object) -> Result<String, SpeechError> {
    let api_base = text_field(litellm_params, "api_base")?;
    let region = text_field(litellm_params, "aws_region_name")?;
    Ok(complete_url(api_base, region))
}

pub struct SignedRequest<'text> {
    pub input: &'text str,
    pub voice: Option<&'text str>,
    pub optional_params: &'text Object,
    pub litellm_params: &'text Object,
    pub now: SystemTime,
}

pub struct TextToSpeechRequest {
    pub url: String,
    pub data: TextToSpeechRequestData,
}

// transform_text_to_speech_request: the body is serialized once, those bytes are signed, and ssml_body carries them
pub fn transform_request(
    request: &SignedRequest<'_>,
    resolver: &dyn CredentialResolver,
) -> Result<TextToSpeechRequest, SpeechError> {
    let url = configured_url(request.litellm_params)?;
    let region = text_field(request.litellm_params, "aws_region_name")?;
    let body = request_body(request.input, request.voice, request.optional_params);
    let payload = serialize(&body);
    let sources = credentials::sources(request.litellm_params)?;
    let resolved = resolver.resolve(&sources)?;
    let headers = sign_post(&SignablePost {
        url: &url,
        region: region.unwrap_or(DEFAULT_REGION),
        body: payload.as_bytes(),
        now: request.now,
        credentials: &resolved,
    })?;
    Ok(TextToSpeechRequest {
        url,
        data: TextToSpeechRequestData {
            dict_body: None,
            ssml_body: Some(payload),
            headers: Some(headers),
        },
    })
}
