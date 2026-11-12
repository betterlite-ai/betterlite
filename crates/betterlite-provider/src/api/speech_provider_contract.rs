use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::speech_provider_contract::{SpeechVoice, TextToSpeechRequestData};
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

pub mod error;

use error::{BaseLlmException, SpeechContractError};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpeechRequest;
pub struct SpeechResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpeechPrepared;

pub type SpeechHeaders = IndexMap<String, String>;

// What `cls.__dict__` holds for one name; descriptors are tagged and never executed
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassAttribute {
    Value(Json),
    Function,
    BuiltinFunction,
    ClassMethod,
    StaticMethod,
    Property,
    CallableObject,
}

// LiteLLM declares voice=None, drop_params=False, kwargs={}; the Default builds a fresh kwargs per call
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeechMappingOptions {
    pub voice: Option<SpeechVoice>,
    pub drop_params: bool,
    pub kwargs: Object,
}

#[derive(Debug, Clone, Copy)]
pub struct SpeechRequestInput<'input> {
    pub model: &'input str,
    pub input: &'input str,
    pub voice: Option<&'input str>,
    pub optional_params: &'input Object,
    pub litellm_params: &'input Object,
    pub headers: &'input SpeechHeaders,
}

pub trait TextToSpeechConfig: Send + Sync {
    fn class_attributes(&self) -> IndexMap<String, ClassAttribute> {
        IndexMap::new()
    }

    fn get_config(&self) -> IndexMap<String, ClassAttribute> {
        let mut kept = self.class_attributes();
        kept.retain(|name, attribute| {
            let reserved = name.starts_with("__") || name.starts_with("_abc");
            let kept_kind = match attribute {
                ClassAttribute::Value(value) => *value != Json::Null,
                ClassAttribute::Property | ClassAttribute::CallableObject => true,
                ClassAttribute::Function
                | ClassAttribute::BuiltinFunction
                | ClassAttribute::ClassMethod
                | ClassAttribute::StaticMethod => false,
            };
            !reserved && kept_kind
        });
        kept
    }

    fn supported_openai_params(&self, model: &str) -> Vec<String>;

    fn map_openai_params(
        &self,
        model: &str,
        optional_params: Object,
        options: &SpeechMappingOptions,
    ) -> (Option<String>, Object);

    fn validate_environment(
        &self,
        _headers: &SpeechHeaders,
        _model: &str,
        _api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> SpeechHeaders {
        SpeechHeaders::new()
    }

    fn complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &Object,
    ) -> Result<String, SpeechContractError> {
        api_base
            .map(str::to_owned)
            .ok_or(SpeechContractError::ApiBaseRequired)
    }

    fn speech_request(&self, request: &SpeechRequestInput<'_>) -> TextToSpeechRequestData;

    fn speech_response(&self, model: &str, raw_response: UpstreamResponse) -> BinaryContent;

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &SpeechHeaders,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: error_message.to_owned(),
            headers: headers.clone(),
        }
    }
}

mod tests;
