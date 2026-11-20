mod scripted;
mod tests;

use std::sync::{Mutex, PoisonError};

use betterlite_core::api::vector_file_provider::{
    OperationKwargs, ensure_provider, prepare_registry_credentials,
};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use scripted::{Input, RecordingLookup};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(answer(input).into_value())
    }
}

// Debug is the only place the helper can leak a credential, and OperationKwargs prints its keys alone
#[derive(Debug)]
struct Answer {
    provider: String,
    kwargs: OperationKwargs,
    lookups: Vec<String>,
    warnings: Vec<String>,
}

impl Answer {
    fn into_value(self) -> serde_json::Value {
        serde_json::json!({
            "provider": self.provider,
            "kwargs": self.kwargs,
            "lookups": self.lookups,
            "warnings": self.warnings,
        })
    }
}

fn answer(input: &Input) -> Answer {
    let provider = ensure_provider(input.custom_llm_provider.as_deref()).to_owned();
    let mut kwargs = input.kwargs.clone();
    let lookups = match input.registry.as_ref() {
        None => Vec::new(),
        Some(script) => {
            let lookup = RecordingLookup {
                script,
                calls: Mutex::new(Vec::new()),
            };
            prepare_registry_credentials(&input.vector_store_id, &mut kwargs, Some(&lookup));
            lookup
                .calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    };
    Answer {
        provider,
        kwargs,
        lookups,
        warnings: Vec::new(),
    }
}
