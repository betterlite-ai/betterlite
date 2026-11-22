use super::backend::{CheapestChatModels, HealthProbeBackend};
use super::calls::ProbeCall;
use super::error::HealthProbeError;
use super::params::{HealthProbeParams, Param};
use super::response::HealthCheckResponse;
use betterlite_models::decisions::Json;
use serde_json::Number;
const CHEAPEST_MODEL_COUNT: usize = 3;
const HEALTH_CHECK_MAX_TOKENS: u8 = 16;
pub async fn probe_wildcard_model(
    _model: &str,
    custom_llm_provider: &str,
    mut params: HealthProbeParams,
    cheapest: &dyn CheapestChatModels,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthCheckResponse, HealthProbeError> {
    let models = cheapest.cheapest_chat_models(custom_llm_provider, CHEAPEST_MODEL_COUNT);
    let Some((first, rest)) = models.split_first() else {
        return Err(HealthProbeError::NoCheapModel {
            provider: custom_llm_provider.to_owned(),
        });
    };
    params.model = Param::Value(first.clone());
    params.fallbacks = Param::Value(fallbacks_value(rest));
    if matches!(params.max_tokens, Param::Absent) {
        params.max_tokens = Param::Value(Json::Number(Number::from(HEALTH_CHECK_MAX_TOKENS)));
    }
    let call = ProbeCall {
        parameters: params.call_parameters(true)?,
        api_key: params.api_key.clone(),
        logging: params.litellm_logging_obj.clone(),
    };
    backend.chat(call).await?;
    Ok(HealthCheckResponse::default())
}
fn fallbacks_value(rest: &[String]) -> Json {
    if rest.is_empty() {
        return Json::Null;
    }
    Json::List(rest.iter().cloned().map(Json::Text).collect())
}
