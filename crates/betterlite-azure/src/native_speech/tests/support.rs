use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use secrecy::{ExposeSecret as _, SecretString};

use crate::native_speech::headers::validate_environment;
use crate::native_speech::identity::NativeSpeechIdentity;
use crate::native_speech::mapping::{
    MappedParams, SpeechKwargs, SpeechOptionalParams, map_openai_params,
};
use crate::native_speech::request::{SpeechRequestData, SpeechTransform, transform_request};

pub(crate) fn map(
    voice: Option<&Json>,
    optional: &SpeechOptionalParams,
    kwargs: &SpeechKwargs,
) -> (Option<String>, MappedParams) {
    map_openai_params(voice, optional, kwargs).expect("mapped speech parameters")
}

pub(crate) fn data(text: &str, voice: Option<&str>, mapped: &MappedParams) -> SpeechRequestData {
    transform_request(SpeechTransform {
        input: text,
        voice,
        mapped,
        headers: BTreeMap::new(),
    })
}

pub(crate) fn speech_body(voice: Option<&str>, lang: Option<&str>) -> (Option<String>, String) {
    let kwargs = SpeechKwargs {
        style: None,
        styledegree: None,
        role: None,
        lang: lang.map(str::to_owned),
    };
    let spelled = voice.map(str::to_owned).map(Json::Text);
    let (name, mapped) = map(spelled.as_ref(), &SpeechOptionalParams::default(), &kwargs);
    let body = data("hi", name.as_deref(), &mapped).ssml_body;
    (name, body)
}

pub(crate) fn identity(api_base: Option<&str>, api_key: &str) -> NativeSpeechIdentity {
    NativeSpeechIdentity {
        api_base: api_base.map(str::to_owned),
        api_key: Some(SecretString::from(api_key)),
    }
}

pub(crate) fn subscription_key(headers: &BTreeMap<String, String>) -> Option<&str> {
    headers.get("Ocp-Apim-Subscription-Key").map(String::as_str)
}

pub(crate) fn cache_key(identity: &NativeSpeechIdentity) -> (String, String) {
    let base = identity.api_base.clone().expect("configured base");
    let key = identity
        .api_key
        .as_ref()
        .map(|value| value.expose_secret().to_owned())
        .expect("configured key");
    (base, key)
}

pub(crate) fn identity_cache(
    identities: &[&NativeSpeechIdentity],
) -> BTreeMap<(String, String), BTreeMap<String, String>> {
    let mut cache = BTreeMap::new();
    for identity in identities {
        cache.insert(
            cache_key(identity),
            validate_environment(identity, BTreeMap::new()),
        );
    }
    cache
}
