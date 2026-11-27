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
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_runner::helpers::{HelperCall, invoke};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::{Value, json};

type Attempt<T> = Result<T, Failure>;

struct NoParameters;

impl SupportedParameterSource for NoParameters {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        Ok(None)
    }
}

struct NoProbe;

impl CompletionBridgeProbe for NoProbe {
    fn bridges_back(&self, _request: &BridgeRequest<'_>) -> Result<bool, ModelInfoError> {
        Ok(false)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(super) struct FamilyFailure {
    message: String,
}

fn failure<E>(error: E) -> Failure
where
    E: Display,
{
    Box::new(FamilyFailure {
        message: error.to_string(),
    })
}

fn call(operation: &str, input: Value) -> Attempt<Value> {
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
        .ok_or_else(|| failure("the nested helper returned no value"))
}

fn part(operation: &str, input: Value, pointer: &str) -> Attempt<Value> {
    let value = call(operation, input)?;
    value
        .pointer(pointer)
        .cloned()
        .ok_or_else(|| failure("the family pass returned no value"))
}

// The provider passes run through the operations that already bind them, so the seam never copies an algorithm.
pub(super) struct Families {
    model_cost: Value,
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
                json!({
                    "model": model,
                    "non_default_params": params,
                    "optional_params": {},
                    "drop_params": false,
                }),
                "/calls/0/optional_params",
            ),
            "transform" => (
                json!({
                    "model": model,
                    "messages": messages,
                    "optional_params": params,
                    "litellm_params": {},
                    "headers": {},
                }),
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
}

#[async_trait::async_trait]
impl BatchFamilies for Families {
    async fn anthropic_params(&self, mapping_model: &str, params: &Params) -> Attempt<Params> {
        let mapped = self.claude("map_params", mapping_model, &[], params)?;
        serde_json::from_value(mapped).map_err(failure)
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
            parameters: &NoParameters,
            probe: &NoProbe,
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
