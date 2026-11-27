use std::convert::Infallible;
use std::fmt::Display;
use std::sync::Arc;

use betterlite_bedrock::batch_records::families::{BatchFamilies, Failure, Params};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_config::sections::local_ttl_cache::Settings as CacheSettings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::file_contracts::is_batch_jsonl_file;
use betterlite_core::api::file_storage_contracts::batch::responses_batch_to_chat_body;
use betterlite_core::api::responses_chat_options::RequestAssembly;
use betterlite_core::api::responses_chat_options::capabilities::CompletionBridgeProbe;
use betterlite_core::responses_bridge::{BridgeRequest, ModelInfoError};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::params::{
    CallType, ChatParameters, ParameterError, ParameterMapping, ParameterPolicy,
    map_listed_parameters,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_runner::helpers::{HelperCall, invoke};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::{Value, json};

use super::fixture::supported_openai_params;

const ALLOWED_OPENAI_PARAMS: &str = "allowed_openai_params";

type Attempt<T> = Result<T, Failure>;

struct NoPorts;

impl SupportedParameterSource for NoPorts {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        Ok(None)
    }
}

impl CompletionBridgeProbe for NoPorts {
    fn bridges_back(&self, _request: &BridgeRequest<'_>) -> Result<bool, ModelInfoError> {
        Ok(false)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(super) struct FamilyFailure {
    message: String,
    pub(super) status: Option<u16>,
}

fn failure<E>(error: E) -> Failure
where
    E: Display,
{
    Box::new(FamilyFailure {
        message: error.to_string(),
        status: None,
    })
}

// An unsupported parameter carries 500 when the encoding failed and 400 otherwise.
fn parameter_failure(error: &ParameterError) -> Failure {
    let encoded = matches!(error, ParameterError::Encode { .. });
    let status = Some(if encoded { 500_u16 } else { 400_u16 });
    Box::new(FamilyFailure {
        message: error.to_string(),
        status,
    })
}

fn part(operation: &str, input: Value, pointer: &str) -> Attempt<Value> {
    let output = invoke(
        &[crate::GROUP],
        &HelperCall {
            operation: operation.to_owned(),
            input,
        },
    );
    if let Some(error) = output.error {
        return Err(failure(error.message));
    }
    output
        .value
        .ok_or_else(|| failure("the nested helper returned no value"))?
        .pointer(pointer)
        .cloned()
        .ok_or_else(|| failure("the family pass returned no value"))
}

// The provider passes run through the operations that already bind them, so the seam never copies an algorithm.
pub(super) struct Families {
    model_cost: Value,
    packaged: Arc<ModelCatalog>,
    cache: DualLocalCache,
}

impl Families {
    pub(super) fn new(model_cost: &ParamMap) -> Result<Self, Box<HelperError>> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
        let cache = DualLocalCache::new(
            None,
            None,
            &CacheSettings::default(),
            Arc::new(|| 0.0_f64),
            Diagnostics {
                redactor: Arc::new(redactor),
                emit: Arc::new(|_event| {}),
            },
        );
        Ok(Self {
            model_cost: serde_json::to_value(model_cost).or_value_error()?,
            packaged: BundledCatalog::default().get(),
            cache,
        })
    }

    fn claude(
        &self,
        function: &str,
        model: &str,
        messages: &[Json],
        params: &Params,
    ) -> Attempt<Value> {
        let (call_input, pointer) = match function {
            "map_params" => (
                json!({"model": model, "non_default_params": params, "optional_params": {}, "drop_params": false}),
                "/calls/0/optional_params",
            ),
            "transform" => (
                json!({"model": model, "messages": messages, "optional_params": params, "litellm_params": {}, "headers": {}}),
                "/calls/0/body",
            ),
            other => return Err(failure(format!("unknown Claude pass {other}"))),
        };
        let input = json!({
            "function": function,
            "model_cost": self.model_cost,
            "calls": [call_input],
        });
        part("bedrock_claude_invoke", input, pointer)
    }

    fn converse_body(&self, model: &str, messages: &[Json], params: &Params) -> Attempt<Json> {
        let map_input = json!({"function": "map_params", "model": model, "model_cost": self.model_cost, "environment": {}, "non_default_params": params, "optional_params": {}, "global_drop": false, "call_drop": false});
        let mapped_params = part("bedrock_converse_params", map_input, "/params")?;
        let request_input = json!({"model": model, "messages": messages, "model_cost": self.model_cost, "optional_params": mapped_params, "litellm_params": {}, "global_drop": false, "modify_params": false, "neutralize_orphaned_blocks": true});
        let body = part("bedrock_converse_request", request_input, "/body")?;
        serde_json::from_value(body).map_err(failure)
    }

    fn chat_mapping(
        &self,
        mapping_model: &str,
        parameters: &ChatParameters,
        policy: &ParameterPolicy,
    ) -> Attempt<ParameterMapping> {
        let supported = supported_openai_params(&self.model_cost, &self.packaged, mapping_model)
            .map_err(|error| failure(error.message))?;
        map_listed_parameters(
            &supported,
            mapping_model,
            CallType::ChatCompletion,
            parameters,
            policy,
        )
        .map_err(|error| parameter_failure(&error))
    }
}

#[async_trait::async_trait]
impl BatchFamilies for Families {
    async fn anthropic_params(&self, mapping_model: &str, params: &Params) -> Attempt<Params> {
        let mut remaining = params.clone();
        let allowed: Vec<String> = match remaining.shift_remove(ALLOWED_OPENAI_PARAMS) {
            Some(Json::Null) | None => Vec::new(),
            Some(value) => {
                let encoded = serde_json::to_value(&value).map_err(failure)?;
                serde_json::from_value(encoded).map_err(failure)?
            }
        };
        let body_encoded = serde_json::to_value(&remaining).map_err(failure)?;
        let parameters: ChatParameters = serde_json::from_value(body_encoded).map_err(failure)?;
        let policy = ParameterPolicy {
            drop_params: false,
            additional_drop_params: Vec::new(),
            allowed_openai_params: allowed,
        };
        let mapping = self.chat_mapping(mapping_model, &parameters, &policy)?;
        let mapped_value = self.claude("map_params", mapping_model, &[], &remaining)?;
        let mut mapped_params: Params = serde_json::from_value(mapped_value).map_err(failure)?;
        for (name, value) in mapping.provider_fields {
            mapped_params.insert(name, serde_json::from_value(value).map_err(failure)?);
        }
        for name in &policy.allowed_openai_params {
            if let Some(value) = remaining.get(name.as_str())
                && !matches!(value, Json::Null)
                && !mapped_params.contains_key(name.as_str())
            {
                mapped_params.insert(name.clone(), value.clone());
            }
        }
        Ok(mapped_params)
    }

    async fn anthropic_body(
        &self,
        model: &str,
        messages: &[Json],
        params: &Params,
    ) -> Attempt<Json> {
        let body = self.claude("transform", model, messages, params)?;
        serde_json::from_value(body).map_err(failure)
    }

    async fn nova_body(&self, model: &str, messages: &[Json], params: &Params) -> Attempt<Json> {
        self.converse_body(model, messages, params)
    }

    async fn responses_to_chat(&self, body: &Json) -> Attempt<Json> {
        let mut assembly = RequestAssembly {
            limits: ApiLimits::default(),
            cache: &self.cache,
            parameters: &NoPorts,
            probe: &NoPorts,
            logging: None,
            route_all_openai_to_responses: false,
        };
        let request = responses_batch_to_chat_body(body, None, &mut assembly).map_err(failure)?;
        let value = serde_json::to_value(&request).map_err(failure)?;
        serde_json::from_value(value).map_err(failure)
    }

    fn is_batch_jsonl(
        &self,
        purpose: Option<&str>,
        content_type: Option<&str>,
        body: Option<&[u8]>,
    ) -> bool {
        is_batch_jsonl_file(purpose, content_type, body)
    }
}
