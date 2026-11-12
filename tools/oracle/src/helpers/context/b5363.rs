use betterlite_context::embedding_optional_params::embedding_optional_params;
use betterlite_context::embedding_optional_params::request::EmbeddingRequest;
use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::error::MapperError;
use betterlite_context::non_default_params::mapper::{DropPolicy, OptionalParamsMapper};
use betterlite_models::controls::json_value::JsonValue;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchInput {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    model: String,
    custom_llm_provider: String,
    user: Option<String>,
    encoding_format: Option<String>,
    dimensions: Option<i64>,
    #[serde(default)]
    kwargs: Params,
    #[serde(default)]
    additional_drop_params: Vec<String>,
    #[serde(default)]
    allowed_openai_params: Vec<String>,
    drop_params: Option<bool>,
    #[serde(default)]
    litellm_drop_params: bool,
    // the Python adapter patches the config selection from this flag; the Rust side gets the answer instead
    #[serde(rename = "force_branch", default)]
    _force_branch: bool,
    provider_config: Option<MapperSpec>,
    mapper: Option<MapperSpec>,
    supported: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MapperSpec {
    supported: Vec<String>,
    #[serde(default)]
    mapped: Params,
}

impl OptionalParamsMapper for MapperSpec {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        self.supported.clone()
    }

    fn map_openai_params(
        &self,
        _non_default_params: &Params,
        mut optional_params: Params,
        _model: &str,
        _drop_params: bool,
    ) -> Result<Params, MapperError> {
        optional_params.extend(self.mapped.clone());
        Ok(optional_params)
    }
}

impl Call {
    fn params(&self) -> Params {
        let mut params = Params::new();
        if let Some(user) = &self.user {
            params.insert("user".to_owned(), JsonValue::Text(user.clone()));
        }
        if let Some(encoding_format) = &self.encoding_format {
            params.insert(
                "encoding_format".to_owned(),
                JsonValue::Text(encoding_format.clone()),
            );
        }
        if let Some(dimensions) = self.dimensions {
            params.insert(
                "dimensions".to_owned(),
                JsonValue::Number(serde_json::Number::from(dimensions)),
            );
        }
        params
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = BatchInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            let params = call.params();
            let request = EmbeddingRequest {
                model: &call.model,
                custom_llm_provider: &call.custom_llm_provider,
                passed_params: &params,
                special_params: &call.kwargs,
                additional_drop_params: &call.additional_drop_params,
                allowed_openai_params: &call.allowed_openai_params,
                drop_policy: DropPolicy {
                    drop_params: call.drop_params,
                    litellm_drop_params: call.litellm_drop_params,
                },
                provider_config: call
                    .provider_config
                    .as_ref()
                    .map(|spec| -> &dyn OptionalParamsMapper { spec }),
                mapper: call
                    .mapper
                    .as_ref()
                    .map(|spec| -> &dyn OptionalParamsMapper { spec }),
                supported: call.supported.as_deref(),
            };
            let result = match embedding_optional_params(&request) {
                Ok(final_params) => json!(final_params),
                Err(error) => json!({ "error": error.status_code() }),
            };
            results.push(result);
        }
        Ok(Value::Array(results))
    }
}
