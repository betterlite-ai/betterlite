use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use secrecy::SecretString;

use crate::native_speech::identity::NativeSpeechIdentity;
use crate::native_speech::mapping::{
    MappedParams, SpeechKwargs, SpeechOptionalParams, map_openai_params,
};
use crate::native_speech::request::{SpeechRequestData, SpeechTransform, transform_request};

pub(crate) fn map(
    voice: Option<&Json>,
    optional: SpeechOptionalParams,
    kwargs: SpeechKwargs,
) -> (Option<String>, MappedParams) {
    map_openai_params(voice, &optional, &kwargs).expect("mapped speech parameters")
}

pub(crate) fn data(text: &str, voice: Option<&str>, mapped: &MappedParams) -> SpeechRequestData {
    transform_request(SpeechTransform {
        input: text,
        voice,
        mapped,
        headers: BTreeMap::new(),
    })
}

pub(crate) fn identity(api_base: Option<&str>, api_key: &str) -> NativeSpeechIdentity {
    NativeSpeechIdentity {
        api_base: api_base.map(str::to_owned),
        api_key: Some(SecretString::from(api_key)),
    }
}
