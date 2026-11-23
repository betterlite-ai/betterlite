use std::collections::BTreeMap;

use betterlite_azure::native_speech::constants::DEFAULT_OUTPUT_FORMAT;
use betterlite_azure::native_speech::endpoint::get_complete_url;
use betterlite_azure::native_speech::error::NativeSpeechError;
use betterlite_azure::native_speech::headers::validate_environment as validate_native_environment;
use betterlite_azure::native_speech::identity::NativeSpeechIdentity;
use betterlite_azure::native_speech::mapping::{
    MappedParams, SpeechKwargs, SpeechOptionalParams, map_openai_params as map_native_openai_params,
};
use betterlite_azure::native_speech::request::{SpeechTransform, transform_request};
use betterlite_azure::speech::dispatch::supported_openai_params;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::speech_provider_contract::{SpeechVoice, TextToSpeechRequestData};
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::speech_provider_contract::error::SpeechContractError;
use crate::api::speech_provider_contract::{
    ClassAttribute, SpeechHeaders, SpeechMappingOptions, SpeechRequestInput, TextToSpeechConfig,
};

// AzureAVATextToSpeechConfig: B538's synthesis behind B2452's text-to-speech contract, reading the mapped dict main.py hands it.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AvaSpeechConfig;

impl TextToSpeechConfig for AvaSpeechConfig {
    // The AVA config carries no class attributes, so replacing the body with Default changes nothing
    #[cfg_attr(test, mutants::skip)]
    fn class_attributes(&self) -> IndexMap<String, ClassAttribute> {
        IndexMap::new()
    }

    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        supported_openai_params()
    }

    fn map_openai_params(
        &self,
        _model: &str,
        optional_params: Object,
        options: &SpeechMappingOptions,
    ) -> (Option<String>, Object) {
        let voice = options.voice.as_ref().and_then(prompt_voice);
        let optional = SpeechOptionalParams {
            response_format: text(&optional_params, "response_format"),
            speed: optional_params
                .get("speed")
                .and_then(|value| value.as_f64()),
        };
        let kwargs = SpeechKwargs {
            style: text(&options.kwargs, "style"),
            styledegree: text(&options.kwargs, "styledegree"),
            role: text(&options.kwargs, "role"),
            lang: text(&options.kwargs, "lang"),
        };
        let (mapped_voice, mapped) = map_native_openai_params(voice.as_ref(), &optional, &kwargs)
            .unwrap_or_else(|_| (None, MappedParams::default()));
        (mapped_voice, strings_object(&mapped_fields(&mapped)))
    }

    fn validate_environment(
        &self,
        headers: &SpeechHeaders,
        _model: &str,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> SpeechHeaders {
        let identity = NativeSpeechIdentity {
            api_base: api_base.map(str::to_owned),
            api_key: api_key.cloned(),
        };
        let caller: BTreeMap<String, String> = headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        validate_native_environment(&identity, caller)
            .into_iter()
            .collect()
    }

    fn complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &Object,
    ) -> Result<String, SpeechContractError> {
        let identity = NativeSpeechIdentity {
            api_base: api_base.map(str::to_owned),
            api_key: None,
        };
        match get_complete_url(&identity) {
            Ok(url) => Ok(url),
            Err(NativeSpeechError::MissingApiBase | NativeSpeechError::SpeedOutOfRange { .. }) => {
                Err(SpeechContractError::ApiBaseRequired)
            }
        }
    }

    fn speech_request(&self, request: &SpeechRequestInput<'_>) -> TextToSpeechRequestData {
        let headers: BTreeMap<String, String> = request
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        let transformed = transform_request(SpeechTransform {
            input: request.input,
            voice: request.voice,
            mapped: &mapped_from(request.optional_params),
            headers,
        });
        TextToSpeechRequestData {
            dict_body: None,
            ssml_body: Some(transformed.ssml_body),
            headers: Some(transformed.headers.into_iter().collect()),
        }
    }

    fn speech_response(&self, _model: &str, raw_response: UpstreamResponse) -> BinaryContent {
        BinaryContent::from_response(raw_response)
    }
}

// the mapped dict the AVA mapping hands the handler: output_format always, the rest only when mapped
fn mapped_from(optional_params: &Object) -> MappedParams {
    MappedParams {
        output_format: text(optional_params, "output_format")
            .unwrap_or_else(|| DEFAULT_OUTPUT_FORMAT.to_owned()),
        rate: text(optional_params, "rate"),
        style: text(optional_params, "style"),
        styledegree: text(optional_params, "styledegree"),
        role: text(optional_params, "role"),
        lang: text(optional_params, "lang"),
    }
}

fn mapped_fields(mapped: &MappedParams) -> IndexMap<String, String> {
    let mut fields =
        IndexMap::from([(String::from("output_format"), mapped.output_format.clone())]);
    for (name, value) in [
        ("rate", &mapped.rate),
        ("style", &mapped.style),
        ("styledegree", &mapped.styledegree),
        ("role", &mapped.role),
        ("lang", &mapped.lang),
    ] {
        if let Some(field) = value {
            fields.insert(name.to_owned(), field.clone());
        }
    }
    fields
}

fn prompt_voice(voice: &SpeechVoice) -> Option<PromptJson> {
    match voice {
        SpeechVoice::Name(name) => Some(PromptJson::Text(name.clone())),
        // LiteLLM maps only a string voice; the native map reads no name off a dict, so it sees none
        SpeechVoice::Options(_) => None,
    }
}

// the mapping hands back only strings, so this conversion cannot fail
fn strings_object(fields: IndexMap<String, String>) -> Object {
    match serde_json::to_value(fields) {
        Ok(value) => serde_json::from_value(value).unwrap_or_default(),
        Err(_error) => Object::new(),
    }
}

fn text(params: &Object, name: &str) -> Option<String> {
    params
        .get(name)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}
