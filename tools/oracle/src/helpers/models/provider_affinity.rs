use betterlite_models::headers::affinity::{
    AffinityParams, add_provider_affinity_header, get_stable_session_id,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::OrHelperError as _;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Add(AddInput),
    StableSession(StableInput),
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct AddInput {
    #[serde(default)]
    pub(crate) headers: IndexMap<String, String>,
    #[serde(default)]
    pub(crate) params: AffinityParams,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct StableInput {
    #[serde(default)]
    pub(crate) params: AffinityParams,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        match input {
            Input::Add(fields) => {
                let headers = add_provider_affinity_header(&fields.headers, &fields.params)
                    .or_value_error()?;
                Ok(json!(headers))
            }
            Input::StableSession(fields) => {
                Ok(json!({"session_id": get_stable_session_id(&fields.params)}))
            }
        }
    }
}
