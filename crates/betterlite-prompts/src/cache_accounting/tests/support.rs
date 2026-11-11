use std::collections::BTreeMap;

use crate::arguments::json::Json;
use crate::cache_accounting::dialect::{DialectInputs, ModelFacts};

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) struct Facts {
    pub(super) flags: BTreeMap<String, bool>,
    pub(super) provider: Option<String>,
}

impl Facts {
    pub(super) fn new(flags: &[(&str, bool)], provider: Option<&str>) -> Self {
        Self {
            flags: flags
                .iter()
                .map(|(model, flag)| ((*model).to_owned(), *flag))
                .collect(),
            provider: provider.map(str::to_owned),
        }
    }
}

impl ModelFacts for Facts {
    fn cache_breakpoint_flag(&self, model: &str) -> Option<bool> {
        self.flags.get(model).copied()
    }

    fn provider(&self, _model: &str) -> Option<String> {
        self.provider.clone()
    }
}

pub(super) fn dialect<'input>(
    model: &'input str,
    environment: &'input BTreeMap<String, String>,
) -> DialectInputs<'input> {
    DialectInputs {
        model,
        custom_llm_provider: None,
        api_base: None,
        base_url: None,
        global_api_base: None,
        environment,
        prompt_cache_options: None,
    }
}
