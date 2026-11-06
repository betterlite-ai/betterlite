use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::anthropic_catalog_lookup::error::CatalogLookupError;
use betterlite_context::anthropic_catalog_lookup::{
    RawCatalog, exact_capability, model_capability, supports_fast_mode,
};
use serde_json::json;

use crate::helpers::HelperError;

mod tests;

#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Candidates {
        model: String,
    },
    Capability {
        model: String,
        key: String,
        runtime: RawCatalog,
        packaged: RawCatalog,
    },
    Exact {
        model: String,
        key: String,
        runtime: RawCatalog,
    },
    FastMode {
        model: String,
        provider: String,
        runtime: RawCatalog,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcome = match input {
            Input::Candidates { model } => {
                CandidatePatterns::new().map(|patterns| json!(patterns.candidates(model)))
            }
            Input::Capability {
                model,
                key,
                runtime,
                packaged,
            } => CandidatePatterns::new()
                .map(|patterns| json!(model_capability(&patterns, runtime, packaged, model, key))),
            Input::Exact {
                model,
                key,
                runtime,
            } => exact_capability(runtime, model, key).map(|flag| json!(flag)),
            Input::FastMode {
                model,
                provider,
                runtime,
            } => supports_fast_mode(runtime, model, provider).map(|enabled| json!(enabled)),
        };
        outcome.map_err(|error| {
            let class = match error {
                CatalogLookupError::MalformedEntry { .. } => "AttributeError",
                CatalogLookupError::Pattern(_) => "error",
            };
            Box::new(HelperError {
                class: class.to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
