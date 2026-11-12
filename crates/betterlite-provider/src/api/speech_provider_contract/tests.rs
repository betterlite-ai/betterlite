#![cfg(test)]

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::speech_provider_contract::{SpeechVoice, TextToSpeechRequestData};
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::arguments::json::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde::Serialize;
use serde_json::Number;

use super::error::{BaseLlmException, SpeechContractError};
use super::{
    ClassAttribute, SpeechHeaders, SpeechMappingOptions, SpeechRequestInput, TextToSpeechConfig,
};

#[derive(Serialize)]
struct Received<'received> {
    optional_params: &'received Object,
    voice: &'received Option<SpeechVoice>,
    drop_params: bool,
    kwargs: &'received Object,
}

struct Base;

impl TextToSpeechConfig for Base {
    fn class_attributes(&self) -> IndexMap<String, ClassAttribute> {
        IndexMap::new()
    }

    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        _model: &str,
        optional_params: Object,
        options: &SpeechMappingOptions,
    ) -> (Option<String>, Object) {
        let received = Received {
            optional_params: &optional_params,
            voice: &options.voice,
            drop_params: options.drop_params,
            kwargs: &options.kwargs,
        };
        let mapped = serde_json::from_str(&serde_json::to_string(&received).unwrap()).unwrap();
        (None, mapped)
    }

    fn speech_request(&self, _request: &SpeechRequestInput<'_>) -> TextToSpeechRequestData {
        TextToSpeechRequestData::default()
    }

    fn speech_response(&self, _model: &str, raw_response: UpstreamResponse) -> BinaryContent {
        BinaryContent::from_response(raw_response)
    }
}

struct Attributes(Vec<(String, ClassAttribute)>);

impl TextToSpeechConfig for Attributes {
    fn class_attributes(&self) -> IndexMap<String, ClassAttribute> {
        self.0
            .iter()
            .map(|(name, attribute)| (name.clone(), attribute.clone()))
            .collect()
    }

    fn supported_openai_params(&self, model: &str) -> Vec<String> {
        Base.supported_openai_params(model)
    }

    fn map_openai_params(
        &self,
        model: &str,
        optional_params: Object,
        options: &SpeechMappingOptions,
    ) -> (Option<String>, Object) {
        Base.map_openai_params(model, optional_params, options)
    }

    fn speech_request(&self, request: &SpeechRequestInput<'_>) -> TextToSpeechRequestData {
        Base.speech_request(request)
    }

    fn speech_response(&self, model: &str, raw_response: UpstreamResponse) -> BinaryContent {
        Base.speech_response(model, raw_response)
    }
}

fn number(value: u64) -> ClassAttribute {
    ClassAttribute::Value(Json::Number(Number::from(value)))
}

fn text(value: &str) -> ClassAttribute {
    ClassAttribute::Value(Json::Text(value.to_owned()))
}

fn names(config: &Attributes) -> Vec<String> {
    config.get_config().into_keys().collect()
}

fn fixture() -> Attributes {
    let own = [
        ("speed", number(0)),
        ("enabled", ClassAttribute::Value(Json::Bool(false))),
        ("label", text("")),
        ("_private", number(1)),
        ("__hidden", number(2)),
        ("_abc_hidden", number(3)),
        ("absent", ClassAttribute::Value(Json::Null)),
        ("method", ClassAttribute::Function),
        ("builtin", ClassAttribute::BuiltinFunction),
        ("class_method", ClassAttribute::ClassMethod),
        ("static_method", ClassAttribute::StaticMethod),
        ("property", ClassAttribute::Property),
        ("callable_object", ClassAttribute::CallableObject),
    ];
    Attributes(
        own.into_iter()
            .map(|(name, attribute)| (name.to_owned(), attribute))
            .collect(),
    )
}

fn headers(pairs: &[(&str, &str)]) -> SpeechHeaders {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn get_config_keeps_falsy_values_underscored_names_properties_and_callables() {
    let config = fixture().get_config();
    let found: Vec<&str> = config.keys().map(String::as_str).collect();
    assert_eq!(
        found,
        [
            "speed",
            "enabled",
            "label",
            "_private",
            "property",
            "callable_object"
        ]
    );
    assert_eq!(config.get("speed"), Some(&number(0)));
    assert_eq!(
        config.get("enabled"),
        Some(&ClassAttribute::Value(Json::Bool(false)))
    );
    assert_eq!(config.get("label"), Some(&text("")));
    assert_eq!(config.get("property"), Some(&ClassAttribute::Property));
}

#[rstest]
#[case::dunder("__hidden")]
#[case::abc_prefix("_abc_hidden")]
#[case::abc_exact("_abc")]
fn get_config_drops_reserved_prefixes(#[case] name: &str) {
    let config = Attributes(vec![
        (name.to_owned(), number(1)),
        ("kept".to_owned(), number(1)),
    ]);
    assert_eq!(names(&config), ["kept"]);
}

#[rstest]
#[case::single_underscore("_")]
#[case::ab_not_abc("_ab")]
#[case::inner_dunder("a__b")]
#[case::inner_abc("x_abc")]
fn get_config_keeps_names_that_only_look_reserved(#[case] name: &str) {
    let config = Attributes(vec![(name.to_owned(), number(1))]);
    assert_eq!(names(&config), [name]);
}

#[rstest]
#[case::function(ClassAttribute::Function)]
#[case::builtin(ClassAttribute::BuiltinFunction)]
#[case::class_method(ClassAttribute::ClassMethod)]
#[case::static_method(ClassAttribute::StaticMethod)]
#[case::none(ClassAttribute::Value(Json::Null))]
fn get_config_drops_callable_wrappers_and_none(#[case] attribute: ClassAttribute) {
    let config = Attributes(vec![("dropped".to_owned(), attribute)]);
    assert!(config.get_config().is_empty());
}

#[test]
fn a_class_without_attributes_has_an_empty_config() {
    assert!(Base.get_config().is_empty());
}

#[test]
fn inherited_values_are_not_part_of_a_subclass_config() {
    let parent = Attributes(vec![("voice".to_owned(), text("alloy"))]);
    let child = Attributes(vec![("speed".to_owned(), number(1))]);
    assert_eq!(names(&parent), ["voice"]);
    assert_eq!(names(&child), ["speed"]);
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

#[test]
fn mapping_options_default_to_no_voice_no_dropping_and_no_kwargs() {
    let options = SpeechMappingOptions::default();
    assert_eq!(options.voice, None);
    assert!(!options.drop_params);
    assert!(options.kwargs.is_empty());
    let (voice, mapped) = Base.map_openai_params("m", Object::new(), &options);
    assert_eq!(voice, None);
    assert_eq!(
        mapped,
        object(r#"{"optional_params":{},"voice":null,"drop_params":false,"kwargs":{}}"#)
    );
}

#[test]
fn explicit_mapping_options_stay_distinguishable_from_the_defaults() {
    let options = SpeechMappingOptions {
        voice: Some(SpeechVoice::Name(String::new())),
        drop_params: true,
        kwargs: object(r#"{"x":1}"#),
    };
    assert_ne!(options, SpeechMappingOptions::default());
    let (_, mapped) = Base.map_openai_params("m", object(r#"{"a":1}"#), &options);
    assert_eq!(
        mapped,
        object(r#"{"optional_params":{"a":1},"voice":"","drop_params":true,"kwargs":{"x":1}}"#)
    );
    let object_voice = SpeechMappingOptions {
        voice: Some(SpeechVoice::Options(Object::new())),
        drop_params: false,
        kwargs: Object::new(),
    };
    let (_, object_mapped) = Base.map_openai_params("m", Object::new(), &object_voice);
    assert_eq!(
        object_mapped,
        object(r#"{"optional_params":{},"voice":{},"drop_params":false,"kwargs":{}}"#)
    );
}

#[test]
fn default_options_do_not_share_kwargs_between_calls() {
    let mut first = SpeechMappingOptions::default();
    first.kwargs.extend(object(r#"{"leaked":true}"#));
    let second = SpeechMappingOptions::default();
    assert!(second.kwargs.is_empty());
    assert!(SpeechMappingOptions::default().kwargs.is_empty());
}

#[test]
fn inherited_validate_environment_returns_an_empty_mapping() {
    let secret = SecretString::from("secret");
    let sent = headers(&[("Authorization", "secret")]);
    assert!(
        Base.validate_environment(&sent, "m", Some(&secret), Some("https://mock"))
            .is_empty()
    );
    assert!(Base.validate_environment(&sent, "m", None, None).is_empty());
}

#[test]
fn inherited_complete_url_rejects_only_a_missing_api_base() {
    let params = Object::new();
    assert_eq!(
        Base.complete_url("m", None, &params),
        Err(SpeechContractError::ApiBaseRequired)
    );
    assert_eq!(Base.complete_url("m", Some(""), &params), Ok(String::new()));
    assert_eq!(
        Base.complete_url("m", Some("https://mock/speech"), &params),
        Ok("https://mock/speech".to_owned())
    );
}

#[test]
fn a_missing_api_base_reads_as_api_base_is_required() {
    assert_eq!(
        SpeechContractError::ApiBaseRequired.to_string(),
        "api_base is required"
    );
}

#[test]
fn error_class_keeps_message_status_and_headers() {
    let error = Base.get_error_class("slow", 429, &headers(&[("retry-after", "2")]));
    assert_eq!(
        error,
        BaseLlmException {
            status_code: 429,
            message: "slow".to_owned(),
            headers: headers(&[("retry-after", "2")]),
        }
    );
    assert_eq!(error.to_string(), "slow");
}

#[test]
fn error_class_keeps_empty_headers_and_an_empty_message() {
    let error = Base.get_error_class("", 500, &SpeechHeaders::new());
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "");
    assert!(error.headers.is_empty());
}

#[test]
fn request_and_response_transforms_dispatch_to_the_implementation() {
    let optional = Object::new();
    let sent = SpeechHeaders::new();
    let request = SpeechRequestInput {
        model: "m",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: &optional,
        headers: &sent,
    };
    assert_eq!(
        Base.speech_request(&request),
        TextToSpeechRequestData::default()
    );
    let raw = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: b"audio".to_vec(),
    };
    let audio = Base.speech_response("m", raw);
    assert_eq!(audio.status, 200);
    assert_eq!(audio.body, b"audio");
}
