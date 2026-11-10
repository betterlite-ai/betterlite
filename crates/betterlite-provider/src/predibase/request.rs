use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::custom::{CustomPromptTemplate, RoleTemplate, custom_prompt};
use betterlite_prompts::factory::{Output, PromptFactory, prompt_factory};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::config::PredibaseConfig;
use super::error::PredibaseSetupError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestBody {
    pub inputs: Option<String>,
    pub parameters: Object,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomPrompt {
    #[serde(default)]
    pub roles: Option<BTreeMap<String, RoleTemplate>>,
    #[serde(default)]
    pub initial_prompt_value: Option<String>,
    #[serde(default)]
    pub final_prompt_value: Option<String>,
}

pub struct RequestContext<'request> {
    pub config: &'request PredibaseConfig,
    pub prompts: &'request PromptFactory,
    pub custom_prompts: &'request BTreeMap<String, CustomPrompt>,
    pub now: DateTime<Utc>,
}

impl RequestContext<'_> {
    pub async fn transform_request(
        &self,
        model: &str,
        messages: &[Message],
        options: &Object,
    ) -> Result<RequestBody, PredibaseSetupError> {
        let prompt = self.prompt(model, messages).await?;
        let mut parameters = options.clone();
        for (name, value) in self.config.get_config() {
            parameters.entry(name).or_insert(value);
        }
        parameters.shift_remove("stream");
        Ok(RequestBody {
            inputs: prompt,
            parameters,
        })
    }

    async fn prompt(
        &self,
        model: &str,
        messages: &[Message],
    ) -> Result<Option<String>, PredibaseSetupError> {
        if let Some(custom) = self.custom_prompts.get(model) {
            return custom_prompt_text(custom, model, messages);
        }
        match prompt_factory(self.prompts, model, messages, None, self.now).await? {
            Output::Text(text) => Ok(Some(text)),
            Output::None => Ok(None),
            Output::Ollama(_) => Err(PredibaseSetupError::OllamaPrompt),
        }
    }
}

fn custom_prompt_text(
    custom: &CustomPrompt,
    model: &str,
    messages: &[Message],
) -> Result<Option<String>, PredibaseSetupError> {
    let Some(roles) = custom.roles.as_ref() else {
        return Err(PredibaseSetupError::CustomPromptField {
            model: model.to_owned(),
            field: "roles",
        });
    };
    let Some(initial) = custom.initial_prompt_value.as_ref() else {
        return Err(PredibaseSetupError::CustomPromptField {
            model: model.to_owned(),
            field: "initial_prompt_value",
        });
    };
    let Some(final_value) = custom.final_prompt_value.as_ref() else {
        return Err(PredibaseSetupError::CustomPromptField {
            model: model.to_owned(),
            field: "final_prompt_value",
        });
    };
    let template = CustomPromptTemplate {
        roles: roles.clone(),
        initial_prompt_value: initial.clone(),
        final_prompt_value: final_value.clone(),
        bos_token: String::new(),
        eos_token: String::new(),
    };
    custom_prompt(&template, messages)
        .map(Some)
        .map_err(PredibaseSetupError::from)
}
