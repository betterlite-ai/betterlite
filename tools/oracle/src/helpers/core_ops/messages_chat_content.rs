use std::future::{Future, ready};

use betterlite_core::api::messages_chat_content::{ChatTarget, translate_messages};
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::OrHelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    messages: Vec<Json>,
    model: Option<String>,
    custom_llm_provider: Option<String>,
    preserve_midturn_system: bool,
    supported: Vec<String>,
}

// LiteLLM reads the capability from its pinned model map, so the case says which models it flags.
struct Supported {
    models: Vec<String>,
}

impl CapabilityPredicates for Supported {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        let flagged = key == NamedCapability::MidConversationSystem.key()
            && self.models.iter().any(|model| model == request.model);
        ready(flagged)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = DecisionJson> + Send {
        ready(DecisionJson::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build()?;
        let mut predicates = Supported {
            models: input.supported.clone(),
        };
        let target = ChatTarget {
            model: input.model.as_deref(),
            custom_llm_provider: input.custom_llm_provider.as_deref(),
        };
        let messages = runtime
            .block_on(translate_messages(
                &input.messages,
                &target,
                input.preserve_midturn_system,
                &mut predicates,
            ))
            .or_value_error()?;
        Ok(json!({ "messages": messages, "input": input.messages }))
    }
}
