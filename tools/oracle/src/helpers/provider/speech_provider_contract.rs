use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::speech_provider_contract::TextToSpeechRequestData;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::arguments::json::Json;
use betterlite_provider::api::speech_provider_contract::{
    ClassAttribute, SpeechHeaders, SpeechMappingOptions, SpeechRequestInput, TextToSpeechConfig,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Number, json};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    RequestContract(RequestContractCase),
    ClassConfiguration,
    BaseUrlAndError(BaseUrlAndErrorCase),
}

#[derive(Debug, Deserialize)]
pub(crate) struct RequestContractCase {
    records: Vec<TextToSpeechRequestData>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BaseUrlAndErrorCase {
    headers: SpeechHeaders,
    api_bases: Vec<Option<String>>,
    error_message: String,
    status_code: u16,
    error_headers: SpeechHeaders,
}

struct Subject;

impl TextToSpeechConfig for Subject {
    fn class_attributes(&self) -> IndexMap<String, ClassAttribute> {
        let value = |count: u64| ClassAttribute::Value(Json::Number(Number::from(count)));
        [
            ("speed", value(0)),
            ("enabled", ClassAttribute::Value(Json::Bool(false))),
            ("label", ClassAttribute::Value(Json::Text(String::new()))),
            ("_private", value(1)),
            ("__hidden", value(2)),
            ("_abc_hidden", value(3)),
            ("_abc", value(4)),
            ("_ab", value(5)),
            ("a__b", value(6)),
            ("absent", ClassAttribute::Value(Json::Null)),
            ("builtin", ClassAttribute::BuiltinFunction),
            ("callable_object", ClassAttribute::CallableObject),
            ("method", ClassAttribute::Function),
            ("class_method", ClassAttribute::ClassMethod),
            ("static_method", ClassAttribute::StaticMethod),
            ("descriptor", ClassAttribute::Property),
        ]
        .into_iter()
        .map(|(name, attribute)| (name.to_owned(), attribute))
        .collect()
    }

    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        _model: &str,
        optional_params: Object,
        _options: &SpeechMappingOptions,
    ) -> (Option<String>, Object) {
        (None, optional_params)
    }

    fn speech_request(&self, _request: &SpeechRequestInput<'_>) -> TextToSpeechRequestData {
        TextToSpeechRequestData::default()
    }

    fn speech_response(&self, _model: &str, raw_response: UpstreamResponse) -> BinaryContent {
        BinaryContent::from_response(raw_response)
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::RequestContract(case) => Ok(request_contract(case)),
            Input::ClassConfiguration => class_configuration(),
            Input::BaseUrlAndError(case) => Ok(base_url_and_error(case)),
        }
    }
}

fn request_contract(case: &RequestContractCase) -> serde_json::Value {
    let defaults = SpeechMappingOptions::default();
    json!({
        "records": case.records,
        "defaults": {
            "voice": defaults.voice,
            "drop_params": defaults.drop_params,
            "kwargs": defaults.kwargs,
        },
    })
}

fn class_configuration() -> HelperResult {
    let mut config = serde_json::Map::new();
    for (name, attribute) in Subject.get_config() {
        let projection = match attribute {
            ClassAttribute::Value(value) => {
                json!({"kind": "value", "value": serde_json::to_value(value).or_value_error()?})
            }
            ClassAttribute::Property => json!({"kind": "property"}),
            ClassAttribute::CallableObject => json!({"kind": "callable"}),
            ClassAttribute::Function
            | ClassAttribute::BuiltinFunction
            | ClassAttribute::ClassMethod
            | ClassAttribute::StaticMethod => continue,
        };
        config.insert(name, projection);
    }
    Ok(json!({"config": config}))
}

fn base_url_and_error(case: &BaseUrlAndErrorCase) -> serde_json::Value {
    let empty = Object::new();
    let urls: Vec<serde_json::Value> = case
        .api_bases
        .iter()
        .map(
            |api_base| match Subject.complete_url("m", api_base.as_deref(), &empty) {
                Ok(url) => json!({"url": url, "error": null}),
                Err(error) => json!({"url": null, "error": error.to_string()}),
            },
        )
        .collect();
    let error = Subject.get_error_class(&case.error_message, case.status_code, &case.error_headers);
    json!({
        "validated": Subject.validate_environment(&case.headers, "m", None, None),
        "urls": urls,
        "error": {
            "message": error.message,
            "status_code": error.status_code,
            "headers": error.headers,
        },
    })
}
