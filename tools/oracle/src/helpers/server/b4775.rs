use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_redact::structured::SensitiveDataMasker;
use betterlite_server::agent_credentials::params::parse_agent_litellm_params;
// ast-grep-ignore: no-long-names
use betterlite_server::agent_credentials::redact::redact_sensitive_agent_litellm_params as redact_params;
// ast-grep-ignore: no-long-names
use betterlite_server::agent_credentials::restore::restore_redacted_litellm_params as restore_params;
use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Redact {
        params: Option<AgentJson>,
    },
    RedactText {
        text: String,
    },
    Parse {
        value: Option<AgentJson>,
    },
    Restore {
        incoming: BTreeMap<String, AgentJson>,
        existing: BTreeMap<String, AgentJson>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let masker = SensitiveDataMasker::with_patterns(Vec::<String>::new(), Vec::<String>::new());
        match input {
            Input::Redact { params } => {
                Ok(redact_params(params.as_ref(), &masker).unwrap_or(AgentJson::Null))
            }
            Input::RedactText { text } => {
                let stored = AgentJson::String(text.clone());
                let Some(AgentJson::String(produced)) = redact_params(Some(&stored), &masker)
                else {
                    return Ok(AgentJson::Null);
                };
                Ok(serde_json::from_str::<AgentJson>(&produced)
                    .unwrap_or_else(|_| AgentJson::String(produced)))
            }
            Input::Parse { value } => Ok(AgentJson::Object(
                parse_agent_litellm_params(value.as_ref())
                    .into_iter()
                    .collect(),
            )),
            Input::Restore { incoming, existing } => Ok(AgentJson::Object(
                restore_params(incoming, existing, &masker)
                    .into_iter()
                    .collect(),
            )),
        }
    }
}
