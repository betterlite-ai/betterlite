use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::eval_create_list::call::EvalsCallParams;

use super::error::EvalsGetUpdateError;

// get_eval's own arguments: the id the path is built from
pub struct GetEvalCall<'call> {
    pub eval_id: &'call str,
    // the arguments @client injects, shared with the create and list operations
    pub params: EvalsCallParams<'call>,
}

// update_eval's own arguments; name and metadata are the only typed request fields
pub struct UpdateEvalCall<'call> {
    pub eval_id: &'call str,
    pub name: Option<&'call str>,
    // filtered against LiteLLM's internal keys before it reaches the request
    pub metadata: Option<&'call IndexMap<String, Json>>,
    // applied after the filtering, so it can put any field, metadata included, on the wire
    pub extra_body: &'call IndexMap<String, Json>,
    pub params: EvalsCallParams<'call>,
}

// one get call together with the logging state its pre-call update writes into
pub struct EvalRetrieval<'call, 'env> {
    pub call: &'call GetEvalCall<'call>,
    pub logging: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

// one update call together with the logging state its pre-call update writes into
pub struct EvalUpdate<'call, 'env> {
    pub call: &'call UpdateEvalCall<'call>,
    pub logging: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'env>,
}

// which operation asks, for the ValueError LiteLLM raises when the manager answers None
#[derive(Clone, Copy)]
pub enum Operation {
    Get,
    Update,
}

impl Operation {
    #[must_use]
    pub fn unsupported(self, provider: &str) -> EvalsGetUpdateError {
        match self {
            Self::Get => EvalsGetUpdateError::GetUnsupported {
                provider: provider.to_owned(),
            },
            Self::Update => EvalsGetUpdateError::UpdateUnsupported {
                provider: provider.to_owned(),
            },
        }
    }
}
