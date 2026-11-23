use betterlite_logging::reservation_binding::{reservation_from_metadata, stamp_callback_bound};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Lookup {
        metadata: IndexMap<String, Json>,
    },
    Stamp {
        litellm_params: IndexMap<String, Json>,
        callback_bound: bool,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Lookup { metadata } => Ok(json!(reservation_from_metadata(metadata))),
            Input::Stamp {
                litellm_params,
                callback_bound,
            } => {
                let mut params = litellm_params.clone();
                stamp_callback_bound(&mut params, *callback_bound);
                Ok(json!(params))
            }
        }
    }
}
