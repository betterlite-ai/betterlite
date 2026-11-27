use std::time::Duration;

use betterlite_context::core_settings::CoreSettings;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::evals::types::{DataSourceConfig, EvalOrder, EvalOrderBy, GraderConfig};
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;

use super::error::EvalsCreateListError;

// the arguments create_eval and list_evals both declare, with the values @client injects kept beside them
pub struct EvalsCallParams<'call> {
    pub extra_headers: Option<&'call Headers>,
    // accepted by both calls; create_eval never forwards it and list_evals overlays it on the pagination
    pub extra_query: &'call IndexMap<String, Json>,
    // LiteLLM's `timeout or request_timeout`: an absent or zero value takes the request timeout
    pub timeout: Option<Duration>,
    // the declared default is "openai"
    pub custom_llm_provider: Option<&'call str>,
    // the remaining keyword arguments GenericLiteLLMParams and the pre-call update read
    pub kwargs: &'call IndexMap<String, Json>,
}

// create_eval's own arguments: the two required fields and the optional name and metadata
pub struct CreateEvalCall<'call> {
    pub data_source_config: DataSourceConfig,
    pub testing_criteria: Vec<GraderConfig>,
    pub name: Option<&'call str>,
    // captured in the call and never forwarded unless extra_body carries a metadata of its own
    pub metadata: Option<&'call IndexMap<String, Json>>,
    pub extra_body: &'call IndexMap<String, Json>,
    pub params: EvalsCallParams<'call>,
}

// list_evals' pagination arguments, each None when the caller did not pass it
pub struct ListEvalsCall<'call> {
    pub limit: Option<i64>,
    pub after: Option<&'call str>,
    pub before: Option<&'call str>,
    pub order: Option<EvalOrder>,
    pub order_by: Option<EvalOrderBy>,
    pub params: EvalsCallParams<'call>,
}

// one create call together with the logging state its pre-call update writes into
pub struct EvalCreation<'call, 'env> {
    pub call: &'call CreateEvalCall<'call>,
    pub logging: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

// one list call together with the logging state its pre-call update writes into
pub struct EvalListing<'call, 'env> {
    pub call: &'call ListEvalsCall<'call>,
    pub logging: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

// LiteLLM writes `timeout or request_timeout`, so an absent or zero timeout takes REQUEST_TIMEOUT
pub fn resolved_timeout(
    timeout: Option<Duration>,
    core: &CoreSettings,
) -> Result<Duration, EvalsCreateListError> {
    match timeout {
        Some(value) if !value.is_zero() => Ok(value),
        Some(_) | None => {
            Duration::try_from_secs_f64(core.request_timeout_seconds).map_err(|source| {
                EvalsCreateListError::Timeout {
                    seconds: core.request_timeout_seconds,
                    source,
                }
            })
        }
    }
}
