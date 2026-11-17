use betterlite_models::decisions::openai::ChoiceValue;
use betterlite_models::decisions::{DecisionsJson, Extra, ir};

use crate::decisions::transformation::provider_config::{DecisionsProviderConfig, first_secret};

use super::PLACEHOLDER_KEY;

pub struct HostedVllmDecisionsConfig;

impl DecisionsProviderConfig for HostedVllmDecisionsConfig {
    const API_KEY_ENV: &'static [&'static str] = &["HOSTED_VLLM_API_KEY"];
    const API_BASE_ENV: &'static [&'static str] = &["HOSTED_VLLM_API_BASE"];
    const API_KEY_REQUIRED: bool = false;

    // resolve_api_key reads the argument first, then the environment, and a key that is only the vLLM placeholder is no key
    fn resolve_api_key(
        &self,
        api_key: Option<&str>,
        get_secret_str: fn(&str) -> String,
    ) -> Option<String> {
        let explicit = api_key.filter(|key| !key.is_empty()).map(str::to_owned);
        let resolved = explicit.or_else(|| first_secret(Self::API_KEY_ENV, &get_secret_str));
        resolved.filter(|key| key != PLACEHOLDER_KEY)
    }

    fn health_check_questions(&self) -> Vec<ir::DecisionQuestion> {
        vec![ir::DecisionQuestion::Choice(ir::ChoiceQuestion {
            name: Some(String::from("reachable")),
            instructions: Some(DecisionsJson::Text(String::from(
                "Is the service reachable?",
            ))),
            choices: vec![
                ir::ChoiceOption {
                    value: ChoiceValue::Text(String::from("yes")),
                    description: None,
                },
                ir::ChoiceOption {
                    value: ChoiceValue::Text(String::from("no")),
                    description: None,
                },
            ],
            extra: Extra::default(),
        })]
    }
}
