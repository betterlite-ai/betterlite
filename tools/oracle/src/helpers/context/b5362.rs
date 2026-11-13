use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::error::MapperError;
use betterlite_context::non_default_params::filters::Endpoint;
use betterlite_context::non_default_params::mapper::OptionalParamsMapper;
use betterlite_context::non_default_params::schema_cleanup::point_refs_at_root_defs;
use betterlite_models::controls::json_value::JsonValue;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::OrHelperError as _;

mod calls;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchInput {
    calls: Vec<Call>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    SelectNonDefault,
    FilterOutLitellm,
    BedrockRoute,
    PreProcess,
    RemoveSensitiveKeys,
    #[serde(rename = "function_fallback")]
    OllamaFallback,
    AddProviderSpecific,
    PeekAliases,
    StripAliases,
    Transcription,
    ImageGeneration,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    passed_params: Params,
    special_params: Params,
    custom_llm_provider: String,
    additional_drop_params: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    #[serde(default)]
    passed_params: Params,
    #[serde(default)]
    default_names: Vec<String>,
    additional_drop_params: Option<Vec<String>>,
    #[serde(default)]
    kwargs: Params,
    #[serde(default)]
    excluding: Vec<String>,
    endpoint: Option<Endpoint>,
    #[serde(default)]
    non_default_params: Params,
    #[serde(default)]
    optional_params: Params,
    model: Option<String>,
    request: Option<Request>,
    remove_sensitive_keys: Option<bool>,
    add_provider_specific_params: Option<bool>,
    #[serde(default)]
    params: Params,
    custom_llm_provider: Option<String>,
    #[serde(default)]
    openai_params: Vec<String>,
    drop_params: Option<bool>,
    litellm_drop_params: Option<bool>,
    mapper: Option<MapperSpec>,
    provider_config: Option<MapperSpec>,
    bedrock: Option<MapperSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MapperSpec {
    supported: Vec<String>,
    fail_status: Option<u16>,
    #[serde(default)]
    records: bool,
    #[serde(default)]
    rewrite_refs: bool,
}

impl OptionalParamsMapper for MapperSpec {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        self.supported.clone()
    }

    fn map_openai_params(
        &self,
        non_default_params: &Params,
        mut optional_params: Params,
        model: &str,
        drop_params: bool,
    ) -> Result<Params, MapperError> {
        if let Some(status_code) = self.fail_status {
            return Err(MapperError {
                status_code,
                message: "fixture mapper failed".to_owned(),
            });
        }
        optional_params.extend(non_default_params.clone());
        if self.records {
            optional_params.insert("seen_drop".to_owned(), JsonValue::Bool(drop_params));
            optional_params.insert("seen_model".to_owned(), JsonValue::Text(model.to_owned()));
        }
        Ok(optional_params)
    }

    fn map_response_format(&self, response_format: &mut JsonValue) {
        if self.rewrite_refs {
            point_refs_at_root_defs(response_format);
        }
    }
}

enum Failure {
    Status(u16),
    Invalid(String),
}

impl From<serde_json::Error> for Failure {
    fn from(error: serde_json::Error) -> Self {
        Self::Invalid(error.to_string())
    }
}

fn required<T>(value: Option<T>, name: &str) -> Result<T, Failure> {
    let Some(present) = value else {
        return Err(Failure::Invalid(format!("a call needs {name}")));
    };
    Ok(present)
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = BatchInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            match calls::run(call) {
                Ok(value) => results.push(value),
                Err(Failure::Status(status)) => results.push(json!({ "error": status })),
                Err(Failure::Invalid(message)) => return Err(message).or_config_error(),
            }
        }
        Ok(Value::Array(results))
    }
}
